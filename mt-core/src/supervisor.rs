//! Connection supervisor: the actor that owns the transport lifecycle,
//! drives the config handshake, reconnects with backoff and dispatches
//! commands to the mesh.

use std::path::PathBuf;
use std::time::Duration;

use meshtastic_protobufs::meshtastic::{NodeInfo, SharedContact, ToRadio, admin_message};
use mt_persistence::{Database, MessageFilter, MessageQuery, now_unix};
use mt_transport::{DeviceAddress, TransportEvent, TransportHandle, spawn_transport};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{self, Instant};

use crate::events::{ConnectionState, CoreCommand, CoreEvent};
use crate::state::MeshState;
use crate::{CoreError, Result};

/// Tunable behaviour of the core.
#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// Directory holding per-device databases.
    pub data_dir: PathBuf,
    /// Reconnect automatically after an unexpected disconnect.
    pub auto_reconnect: bool,
    /// How often to send a keep-alive heartbeat when connected.
    pub heartbeat_interval: Duration,
    /// How long to wait for the config handshake before retrying it.
    pub handshake_timeout: Duration,
    /// How long an outbound message may stay unacknowledged.
    pub message_timeout: Duration,
    /// Number of historical messages loaded per device connection.
    pub history_page: u32,
    /// Upper bound on reconnect backoff.
    pub max_reconnect_delay: Duration,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            data_dir: Database::default_dir().unwrap_or_else(|_| PathBuf::from(".")),
            auto_reconnect: true,
            heartbeat_interval: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(20),
            message_timeout: Duration::from_secs(90),
            history_page: 300,
            max_reconnect_delay: Duration::from_secs(30),
        }
    }
}

enum SessionOutcome {
    Idle,
    Reconnect,
    SwitchDevice(DeviceAddress),
    Shutdown,
}

enum IdleOutcome {
    Connect(DeviceAddress),
    Shutdown,
}

enum BackoffOutcome {
    Retry,
    Connect(DeviceAddress),
    Idle,
    Shutdown,
}

/// The core actor. One instance lives for the whole application lifetime.
pub(crate) struct Supervisor {
    pub(crate) cfg: CoreConfig,
    pub(crate) event_tx: broadcast::Sender<CoreEvent>,
    pub(crate) cmd_rx: mpsc::Receiver<CoreCommand>,

    pub(crate) state: MeshState,
    pub(crate) db: Option<Database>,
    pub(crate) db_node: Option<u32>,

    pub(crate) transport: Option<TransportHandle>,
    pub(crate) conn_state: ConnectionState,
    pub(crate) desired: Option<DeviceAddress>,
    /// Session passkey echoed by the firmware in admin responses; required
    /// by newer firmware for subsequent admin writes.
    pub(crate) session_passkey: Vec<u8>,

    reconnect_attempt: u32,
    session_connected: bool,
    pub(crate) handshake_complete: bool,
    handshake_stage: u8,
    handshake_retried: bool,
    handshake_deadline: Instant,
    next_heartbeat: Instant,
}

impl Supervisor {
    pub(crate) fn new(
        cfg: CoreConfig,
        event_tx: broadcast::Sender<CoreEvent>,
        cmd_rx: mpsc::Receiver<CoreCommand>,
    ) -> Self {
        Self {
            cfg,
            event_tx,
            cmd_rx,
            state: MeshState::default(),
            db: None,
            db_node: None,
            transport: None,
            conn_state: ConnectionState::Disconnected,
            desired: None,
            session_passkey: Vec::new(),
            reconnect_attempt: 0,
            session_connected: false,
            handshake_complete: false,
            handshake_stage: 0,
            handshake_retried: false,
            handshake_deadline: Instant::now(),
            next_heartbeat: Instant::now(),
        }
    }

    /// Actor entry point.
    pub(crate) async fn run(mut self) {
        let mut pending_connect: Option<DeviceAddress> = None;

        loop {
            let addr = match pending_connect.take() {
                Some(addr) => addr,
                None => match self.idle_wait().await {
                    IdleOutcome::Connect(addr) => addr,
                    IdleOutcome::Shutdown => break,
                },
            };
            self.desired = Some(addr.clone());

            match self.connection_session(addr).await {
                SessionOutcome::Shutdown => break,
                SessionOutcome::SwitchDevice(addr) => pending_connect = Some(addr),
                SessionOutcome::Idle => {
                    self.desired = None;
                    self.publish_state(ConnectionState::Disconnected);
                }
                SessionOutcome::Reconnect => {
                    if !self.cfg.auto_reconnect {
                        self.publish_state(ConnectionState::Disconnected);
                        continue;
                    }
                    match self.backoff_wait().await {
                        BackoffOutcome::Retry => {
                            pending_connect = self.desired.clone();
                        }
                        BackoffOutcome::Connect(addr) => pending_connect = Some(addr),
                        BackoffOutcome::Idle => self.desired = None,
                        BackoffOutcome::Shutdown => break,
                    }
                }
            }
        }

        self.shutdown_transport().await;
        tracing::debug!("core supervisor stopped");
    }

    /// Wait for a connect/shutdown command while idle.
    async fn idle_wait(&mut self) -> IdleOutcome {
        loop {
            match self.cmd_rx.recv().await {
                None | Some(CoreCommand::ShutdownCore) => return IdleOutcome::Shutdown,
                Some(CoreCommand::Connect(addr)) => return IdleOutcome::Connect(addr),
                Some(CoreCommand::Disconnect) => {
                    self.publish_state(ConnectionState::Disconnected);
                }
                Some(cmd) => self.dispatch_command(cmd).await,
            }
        }
    }

    /// Sleep out the reconnect backoff, staying responsive to commands.
    async fn backoff_wait(&mut self) -> BackoffOutcome {
        self.reconnect_attempt = self.reconnect_attempt.saturating_add(1);
        let delay = self.backoff_delay();
        if let Some(addr) = self.desired.clone() {
            self.publish_state(ConnectionState::Reconnecting {
                address: addr,
                attempt: self.reconnect_attempt,
                delay,
            });
        }

        let deadline = Instant::now() + delay;
        loop {
            tokio::select! {
                _ = time::sleep_until(deadline) => return BackoffOutcome::Retry,
                cmd = self.cmd_rx.recv() => match cmd {
                    None | Some(CoreCommand::ShutdownCore) => return BackoffOutcome::Shutdown,
                    Some(CoreCommand::Connect(addr)) => return BackoffOutcome::Connect(addr),
                    Some(CoreCommand::Disconnect) => {
                        self.publish_state(ConnectionState::Disconnected);
                        return BackoffOutcome::Idle;
                    }
                    Some(cmd) => self.dispatch_command(cmd).await,
                }
            }
        }
    }

    fn backoff_delay(&self) -> Duration {
        let shift = self.reconnect_attempt.saturating_sub(1).min(5);
        let delay = Duration::from_secs(1u64 << shift);
        delay.min(self.cfg.max_reconnect_delay)
    }

    /// One connection attempt: open the transport, handshake, serve the
    /// mesh until it drops or the user acts.
    async fn connection_session(&mut self, addr: DeviceAddress) -> SessionOutcome {
        self.publish_state(ConnectionState::Connecting(addr.clone()));
        self.state = MeshState::default();
        self.db = None;
        self.db_node = None;
        self.session_connected = false;
        self.handshake_complete = false;
        self.handshake_stage = 0;
        self.handshake_deadline = Instant::now() + self.cfg.handshake_timeout;
        self.next_heartbeat = Instant::now() + self.cfg.heartbeat_interval;
        self.session_passkey.clear();

        let (handle, mut events) = spawn_transport(addr.clone());
        self.transport = Some(handle);

        let mut ticker = time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

        let outcome = loop {
            tokio::select! {
                biased;

                cmd = self.cmd_rx.recv() => match cmd {
                    None | Some(CoreCommand::ShutdownCore) => break SessionOutcome::Shutdown,
                    Some(CoreCommand::Disconnect) => {
                        self.publish_state(ConnectionState::Disconnecting(addr.clone()));
                        self.shutdown_transport().await;
                        break SessionOutcome::Idle;
                    }
                    Some(CoreCommand::Connect(next)) => {
                        self.shutdown_transport().await;
                        break SessionOutcome::SwitchDevice(next);
                    }
                    Some(cmd) => self.dispatch_command(cmd).await,
                },

                event = events.recv() => match event {
                    None => break SessionOutcome::Reconnect,
                    Some(TransportEvent::Connected) => {
                        self.session_connected = true;
                        self.publish_state(ConnectionState::Handshaking(addr.clone()));
                        let _ = self
                            .send_to_radio(mt_protocol::builders::initial_handshake())
                            .await;
                        self.handshake_stage = 1;
                        self.handshake_retried = false;
                        self.handshake_deadline = Instant::now() + self.cfg.handshake_timeout;
                    }
                    Some(TransportEvent::FromRadio(msg)) => self.handle_from_radio(msg).await,
                    Some(TransportEvent::DeviceLog(line)) => self.emit(CoreEvent::DeviceLog(line)),
                    Some(TransportEvent::BlePairingRequest { address }) => {
                        self.emit(CoreEvent::BlePairingRequest { address })
                    }
                    Some(TransportEvent::Disconnected { error }) => {
                        tracing::info!(?error, "transport disconnected");
                        if let Some(err) = &error {
                            self.emit(CoreEvent::Error(format!("connection lost: {err}")));
                        }
                        break SessionOutcome::Reconnect;
                    }
                },

                _ = ticker.tick() => {
                    self.on_tick().await;
                }
            }
        };

        self.transport = None;
        self.session_connected = false;
        self.handshake_complete = false;
        self.requeue_inflight();
        outcome
    }

    /// Periodic maintenance: handshake watchdog, heartbeat, message timeouts.
    async fn on_tick(&mut self) {
        if self.session_connected && !self.handshake_complete {
            if Instant::now() >= self.handshake_deadline {
                if !self.handshake_retried {
                    self.handshake_retried = true;
                    self.handshake_deadline = Instant::now() + self.cfg.handshake_timeout;
                    let probe = if self.handshake_stage >= 2 {
                        mt_protocol::builders::second_handshake()
                    } else {
                        mt_protocol::builders::initial_handshake()
                    };
                    tracing::debug!(
                        stage = self.handshake_stage,
                        "handshake slow; resending probe"
                    );
                    let _ = self.send_to_radio(probe).await;
                } else {
                    tracing::warn!("handshake timed out; reconnecting");
                    self.emit(CoreEvent::Error(
                        "device did not complete the config handshake".into(),
                    ));
                    if let Some(t) = &self.transport {
                        t.disconnect().await;
                    }
                }
            }
            return;
        }

        if self.handshake_complete {
            if Instant::now() >= self.next_heartbeat {
                self.next_heartbeat = Instant::now() + self.cfg.heartbeat_interval;
                let _ = self.send_to_radio(mt_protocol::builders::heartbeat()).await;
            }
            self.sweep_message_timeouts();
        }
    }

    // command dispatch

    /// Execute a command. Commands that need a live mesh report an error
    /// when the handshake has not completed.
    pub(crate) async fn dispatch_command(&mut self, cmd: CoreCommand) {
        use CoreCommand as C;
        let result: Result<()> = match cmd {
            C::ShutdownCore | C::Connect(_) | C::Disconnect => Ok(()),

            C::SendText {
                text,
                channel,
                to,
                reply_id,
            } => self.send_text(text, channel, to, reply_id).await,

            C::RequestPosition(num) => {
                self.send_online(mt_protocol::builders::position_request(num))
                    .await
            }
            C::Traceroute(num) => {
                self.send_online(mt_protocol::builders::traceroute_request(num))
                    .await
            }
            C::RequestNodeList(num) => {
                self.send_online(mt_protocol::builders::get_device_metadata_request(num))
                    .await
            }

            C::SetFavorite { node_num, favorite } => self.set_favorite(node_num, favorite),
            C::SetIgnored { node_num, ignored } => self.set_ignored(node_num, ignored),
            C::RemoveNode(node_num) => self.remove_node(node_num),
            C::AddContact(contact) => {
                // Mirror the contact locally (the radio adds it to its node
                // database), then ask the device to store it. The device then
                // propagates the key to the mesh through its NodeInfo.
                if let Some(node) = self.upsert_contact(&contact) {
                    if let Some(db) = &self.db {
                        let _ = db.upsert_node(&node);
                    }
                    self.emit(CoreEvent::Node(Box::new(node)));
                }
                let msg = self.admin_radio(
                    admin_message::PayloadVariant::AddContact(*contact),
                    false,
                    mt_protocol::constants::BROADCAST_ADDR,
                );
                self.send_online(msg).await
            }

            C::SetOwner {
                long_name,
                short_name,
                is_licensed,
            } => {
                // Preserve the local node's other identity fields (role, key,
                // hardware model) and reflect the new names immediately, so the
                // UI updates without waiting for a reconnect.
                let mut user = self
                    .state
                    .my_num()
                    .and_then(|my| self.state.nodes.get(&my))
                    .and_then(|node| node.user.clone())
                    .unwrap_or_default();
                user.long_name = long_name;
                user.short_name = short_name;
                user.is_licensed = is_licensed;

                if let Some(my) = self.state.my_num() {
                    if let Some(mut node) = self.state.nodes.get(&my).cloned() {
                        node.user = Some(user.clone());
                        let node = self.state.upsert_node(node);
                        if let Some(db) = &self.db {
                            let _ = db.upsert_node(&node);
                        }
                        self.emit(CoreEvent::Node(Box::new(node)));
                    }
                }

                let msg = self.admin_radio(
                    admin_message::PayloadVariant::SetOwner(user),
                    false,
                    mt_protocol::constants::BROADCAST_ADDR,
                );
                self.send_online(msg).await
            }

            C::SetChannel(channel) => {
                let msg = self.admin_radio(
                    admin_message::PayloadVariant::SetChannel(*channel),
                    false,
                    mt_protocol::constants::BROADCAST_ADDR,
                );
                self.apply_admin_edit(msg).await
            }
            C::SetConfig(config) => {
                let msg = self.admin_radio(
                    admin_message::PayloadVariant::SetConfig(*config),
                    false,
                    mt_protocol::constants::BROADCAST_ADDR,
                );
                self.apply_admin_edit(msg).await
            }
            C::SetModuleConfig(config) => {
                let msg = self.admin_radio(
                    admin_message::PayloadVariant::SetModuleConfig(*config),
                    false,
                    mt_protocol::constants::BROADCAST_ADDR,
                );
                self.apply_admin_edit(msg).await
            }
            C::SetTime(seconds) => {
                self.send_to_radio(mt_protocol::builders::set_time_only(seconds))
                    .await
            }
            C::SetFixedPosition(position) => {
                let msg = self.admin_radio(
                    admin_message::PayloadVariant::SetFixedPosition(position),
                    false,
                    mt_protocol::constants::BROADCAST_ADDR,
                );
                self.apply_admin_edit(msg).await
            }

            C::Resync => {
                self.handshake_complete = false;
                self.handshake_stage = 0;
                self.handshake_deadline = Instant::now() + self.cfg.handshake_timeout;
                if let Some(addr) = self.desired.clone() {
                    self.publish_state(ConnectionState::Handshaking(addr));
                }
                self.send_online(mt_protocol::builders::initial_handshake())
                    .await
            }

            C::Reboot { dest, seconds } => {
                let msg = self.admin_radio(
                    admin_message::PayloadVariant::RebootSeconds(seconds),
                    false,
                    dest,
                );
                self.send_online(msg).await
            }
            C::Shutdown { dest, seconds } => {
                let msg = self.admin_radio(
                    admin_message::PayloadVariant::ShutdownSeconds(seconds),
                    false,
                    dest,
                );
                self.send_online(msg).await
            }
            C::FactoryReset { dest, full_device } => {
                let variant = if full_device {
                    admin_message::PayloadVariant::FactoryResetDevice(1)
                } else {
                    admin_message::PayloadVariant::FactoryResetConfig(1)
                };
                let msg = self.admin_radio(variant, false, dest);
                self.send_online(msg).await
            }
            C::SetAutoReconnect(enabled) => {
                self.cfg.auto_reconnect = enabled;
                Ok(())
            }
            C::SubmitBlePasskey(passkey) => {
                tracing::debug!(passkey, "forwarding ble passkey to transport");
                if let Some(transport) = &self.transport {
                    transport.submit_ble_passkey(passkey).await;
                }
                Ok(())
            }
        };

        if let Err(err) = result {
            self.emit(CoreEvent::Error(err.to_string()));
        }
    }

    /// Send a message that requires a completed handshake.
    async fn send_online(&mut self, msg: ToRadio) -> Result<()> {
        if !self.handshake_complete {
            return Err(CoreError::NotConnected);
        }
        self.send_to_radio(msg).await
    }

    /// Build an admin packet, injecting the current session passkey.
    fn admin_radio(
        &self,
        variant: admin_message::PayloadVariant,
        want_response: bool,
        dest: u32,
    ) -> ToRadio {
        let mut admin = mt_protocol::builders::admin(variant);
        admin.session_passkey = self.session_passkey.clone();
        mt_protocol::builders::admin_packet(dest, admin, want_response)
    }

    /// Apply an admin change inside a begin/commit edit session.
    async fn apply_admin_edit(&mut self, set: ToRadio) -> Result<()> {
        if !self.handshake_complete {
            return Err(CoreError::NotConnected);
        }
        let _ = self
            .send_to_radio(mt_protocol::builders::begin_edit_settings())
            .await;
        time::sleep(Duration::from_millis(150)).await;
        self.send_to_radio(set).await?;
        time::sleep(Duration::from_millis(150)).await;
        let _ = self
            .send_to_radio(mt_protocol::builders::commit_edit_settings())
            .await;
        Ok(())
    }

    fn set_favorite(&mut self, node_num: u32, favorite: bool) -> Result<()> {
        let updated = self.state.nodes.get_mut(&node_num).map(|node| {
            node.is_favorite = favorite;
            node.clone()
        });
        if let Some(node) = updated {
            self.emit(CoreEvent::Node(Box::new(node)));
        }
        if let Some(db) = &self.db {
            let _ = db.set_favorite(node_num, favorite);
        }
        let variant = if favorite {
            admin_message::PayloadVariant::SetFavoriteNode(node_num)
        } else {
            admin_message::PayloadVariant::RemoveFavoriteNode(node_num)
        };
        self.send_admin(variant)
    }

    fn set_ignored(&mut self, node_num: u32, ignored: bool) -> Result<()> {
        let updated = self.state.nodes.get_mut(&node_num).map(|node| {
            node.is_ignored = ignored;
            node.clone()
        });
        if let Some(node) = updated {
            self.emit(CoreEvent::Node(Box::new(node)));
        }
        if let Some(db) = &self.db {
            let _ = db.set_ignored(node_num, ignored);
        }
        let variant = if ignored {
            admin_message::PayloadVariant::SetIgnoredNode(node_num)
        } else {
            admin_message::PayloadVariant::RemoveIgnoredNode(node_num)
        };
        self.send_admin(variant)
    }

    /// Merge a shared contact into the in-memory node database, creating the
    /// node when it is new. Returns the stored node.
    fn upsert_contact(&mut self, contact: &SharedContact) -> Option<NodeInfo> {
        let user = contact.user.clone()?;
        let mut node = self
            .state
            .nodes
            .get(&contact.node_num)
            .cloned()
            .unwrap_or(NodeInfo {
                num: contact.node_num,
                ..Default::default()
            });
        node.num = contact.node_num;
        node.user = Some(user);
        if contact.should_ignore {
            node.is_ignored = true;
        }
        Some(self.state.upsert_node(node))
    }

    fn remove_node(&mut self, node_num: u32) -> Result<()> {
        self.state.nodes.remove(&node_num);
        if let Some(db) = &self.db {
            let _ = db.remove_node(node_num);
        }
        self.emit(CoreEvent::NodeRemoved(node_num));
        // Remove it from the device's node database too, otherwise it comes
        // back on the next handshake.
        if self.handshake_complete {
            let _ = self.send_admin(admin_message::PayloadVariant::RemoveByNodenum(node_num));
        }
        Ok(())
    }

    fn send_admin(&self, variant: admin_message::PayloadVariant) -> Result<()> {
        if !self.handshake_complete {
            return Err(CoreError::NotConnected);
        }
        let msg = self.admin_radio(variant, false, mt_protocol::constants::BROADCAST_ADDR);
        // Fire and forget: this method is sync, so hand the frame to the
        // transport through a fresh task to avoid blocking callers.
        if let Some(handle) = self.transport.clone() {
            tokio::spawn(async move {
                let _ = handle.send(msg).await;
            });
        }
        Ok(())
    }

    // helpers

    /// Send a decoded frame over the transport.
    pub(crate) async fn send_to_radio(&self, msg: ToRadio) -> Result<()> {
        match &self.transport {
            Some(handle) => handle.send(msg).await.map_err(CoreError::Transport),
            None => Err(CoreError::NotConnected),
        }
    }

    /// Publish a connection-state transition.
    pub(crate) fn publish_state(&mut self, state: ConnectionState) {
        self.conn_state = state.clone();
        self.emit(CoreEvent::Connection(state));
    }

    /// Broadcast an event to subscribers (ignoring "no receivers").
    pub(crate) fn emit(&self, event: CoreEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Ask the transport to close and wait briefly for the task to end.
    async fn shutdown_transport(&mut self) {
        if let Some(handle) = self.transport.take() {
            handle.disconnect().await;
            // Drop the event receiver we hold (if any) by leaving the
            // session; give the socket a moment to close politely.
            time::sleep(Duration::from_millis(60)).await;
        }
        self.session_connected = false;
        self.handshake_complete = false;
    }

    /// Open the per-device database and load its contents into memory.
    pub(crate) fn open_device_db(&mut self, node_num: u32) {
        match Database::open_for_device(&self.cfg.data_dir, node_num) {
            Ok(db) => {
                self.db = Some(db);
                self.db_node = Some(node_num);
                self.load_persisted();
            }
            Err(err) => self.emit(CoreEvent::Error(format!(
                "failed to open the device database: {err}"
            ))),
        }
    }

    fn load_persisted(&mut self) {
        let Some(db) = self.db.clone() else {
            return;
        };

        match db.list_nodes() {
            Ok(nodes) => {
                for node in nodes {
                    self.state.nodes.insert(node.num, node.clone());
                    self.emit(CoreEvent::Node(Box::new(node)));
                }
            }
            Err(err) => self.emit(CoreEvent::Error(format!("reading nodes failed: {err}"))),
        }

        if let Ok(channels) = db.list_channels() {
            for channel in channels {
                self.state.apply_channel(channel.clone());
                self.emit(CoreEvent::Channel(Box::new(channel)));
            }
        }
        if let Ok(configs) = db.list_configs() {
            for config in configs {
                self.state.apply_config(config.clone());
                self.emit(CoreEvent::Config(Box::new(config)));
            }
        }
        if let Ok(configs) = db.list_module_configs() {
            for config in configs {
                self.state.apply_module_config(config.clone());
                self.emit(CoreEvent::ModuleConfig(Box::new(config)));
            }
        }

        let query = MessageQuery {
            filter: MessageFilter::All,
            limit: self.cfg.history_page,
            before_id: None,
        };
        match db.list_messages(&query) {
            Ok(messages) => self.emit(CoreEvent::MessagesLoaded(messages)),
            Err(err) => self.emit(CoreEvent::Error(format!("reading messages failed: {err}"))),
        }
    }

    /// Handle a `config_complete_id` sentinel, driving the two-stage handshake.
    ///
    /// Stage 1 (`HANDSHAKE_NONCE_1`) streams config, channels and the file
    /// manifest but skips the node database. Stage 2 (`HANDSHAKE_NONCE_2`)
    /// streams the node database. Between the stages the firmware wants a
    /// heartbeat to settle before the NodeDB burst.
    pub(crate) async fn on_config_complete(&mut self, nonce: u32) {
        if nonce == mt_protocol::constants::HANDSHAKE_NONCE_1 {
            if self.handshake_stage < 2 {
                tracing::debug!("config stage complete; requesting node database");
                self.handshake_stage = 2;
                self.handshake_retried = false;
                self.handshake_deadline = Instant::now() + self.cfg.handshake_timeout;
                let _ = self.send_to_radio(mt_protocol::builders::heartbeat()).await;
                tokio::time::sleep(Duration::from_millis(100)).await;
                let _ = self
                    .send_to_radio(mt_protocol::builders::second_handshake())
                    .await;
            }
            return;
        }

        // Stage 2 sentinel, or a legacy single-stage nonce: we are connected.
        self.on_handshake_complete(nonce).await;
    }

    /// Called once the handshake completes.
    pub(crate) async fn on_handshake_complete(&mut self, nonce: u32) {
        if self.handshake_complete {
            return;
        }
        self.handshake_complete = true;
        self.state.handshake_nonce = Some(nonce);
        self.reconnect_attempt = 0;
        self.next_heartbeat = Instant::now() + self.cfg.heartbeat_interval;

        // Hand the radio the host clock, like the official clients do.
        // Radios without GPS or an RTC otherwise leave `rx_time` at zero on
        // received packets, so their node database never refreshes the
        // `last_heard` timestamps and every reconnect reloads stale values.
        let _ = self
            .send_to_radio(mt_protocol::builders::set_time_only(now_unix() as u32))
            .await;

        // Persist the channel snapshot now that all slots are in.
        if let Some(db) = &self.db {
            if let Err(err) = db.replace_channels(&self.state.channels) {
                self.emit(CoreEvent::Error(format!("saving channels failed: {err}")));
            }
            let _ = db.requeue_inflight();
        }
        self.state.outbound.clear();

        let node_num = self.state.my_num().unwrap_or(0);
        if let Some(addr) = self.desired.clone() {
            self.publish_state(ConnectionState::Connected {
                address: addr,
                node_num,
            });
        }

        self.resend_pending().await;
        tracing::info!(node_num, "handshake complete");
    }
}
