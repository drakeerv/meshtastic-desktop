//! Application state and update logic.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use iced::widget::{button, column, container, markdown, row, text};
use iced::{Element, Length, Padding, Size, Subscription, Task, Theme};
use iced::{keyboard, window};
use meshtastic_protobufs::meshtastic::{
    Channel, Config, DeviceMetadata, ModuleConfig, MyNodeInfo, NodeInfo, Position, SharedContact,
    Telemetry,
};
use mt_core::{
    ConnectionState, CoreCommand, CoreEvent, CoreHandle, DeviceAddress, DiscoveredDevice,
    DiscoveryEvent, MessageRecord, TransportKind,
};
use mt_transport::DiscoveryHandle;

use crate::bridge::Bridge;
use crate::config_editor::{ChannelEditor, Editor, Section, SectionValue};
use crate::map::{MapNode, MapView};
use crate::settings::{AppSettings, ThemePref};
use crate::tiles::{self, TileKey};
use crate::tray::{TrayEvent, TrayHandle, TrayState};
use crate::views;
use crate::{format, theme};

/// Firmware text-message limit, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 200;

/// How long a transient notice stays on screen before fading away.
const NOTICE_TTL: std::time::Duration = std::time::Duration::from_secs(6);

/// A transient status message shown at the bottom of the window.
#[derive(Debug, Clone)]
pub struct Notice {
    pub text: String,
    expires: Instant,
}

/// The five primary sections, shown in the navigation rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Messages,
    Nodes,
    Map,
    Connect,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Messages,
        Tab::Nodes,
        Tab::Map,
        Tab::Connect,
        Tab::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Messages => "Messages",
            Tab::Nodes => "Nodes",
            Tab::Map => "Map",
            Tab::Connect => "Connect",
            Tab::Settings => "Settings",
        }
    }
}

/// Which page of the Settings section is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    /// Category picker.
    Hub,
    /// Client preferences (no device needed).
    App,
    /// Owner, radio/module config and device actions (device needed).
    Device,
    /// The eight channel slots (device needed).
    Channels,
}

/// Which conversation the Messages view is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversation {
    Channel(u32),
    Peer(u32),
}

/// Which transport the Connect view is filtered to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectTab {
    All,
    Serial,
    Bluetooth,
    Ip,
}

impl ConnectTab {
    pub const ALL: [ConnectTab; 4] = [
        ConnectTab::All,
        ConnectTab::Serial,
        ConnectTab::Bluetooth,
        ConnectTab::Ip,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConnectTab::All => "All",
            ConnectTab::Serial => "Serial",
            ConnectTab::Bluetooth => "Bluetooth",
            ConnectTab::Ip => "IP",
        }
    }
}

/// Latest traceroute result, kept for the node detail panel.
#[derive(Debug, Clone)]
pub struct TracerouteInfo {
    pub from: u32,
    pub route: Vec<u32>,
    pub snr_towards: Vec<i32>,
    pub route_back: Vec<u32>,
    pub snr_back: Vec<i32>,
    pub at: i64,
}

/// A contact link rendered as a QR code, ready to display.
pub struct ContactQr {
    pub title: String,
    /// The `https://meshtastic.org/v/#…` link, shown and copyable.
    pub uri: String,
    /// The encoded QR code. Cached because building it is not free.
    pub data: iced::widget::qr_code::Data,
}

/// A contact decoded from a pasted link, shown before import.
#[derive(Debug, Clone)]
pub struct ContactPreview {
    pub name: String,
    pub id: String,
}

/// State of the import-contact dialog.
#[derive(Debug, Clone, Default)]
pub struct ContactImport {
    /// The pasted link or payload.
    pub input: String,
    /// Why the input could not be parsed, if it could not.
    pub error: Option<String>,
    /// A short summary of the decoded contact, when valid.
    pub preview: Option<ContactPreview>,
    /// The decoded contact, ready to dispatch.
    pub contact: Option<SharedContact>,
}

/// An in-progress BLE pairing prompt.
#[derive(Debug, Clone)]
pub struct BlePairing {
    /// The device asking to be paired.
    pub address: String,
    /// The passkey the user is typing.
    pub input: String,
}

/// All UI messages.
#[derive(Debug, Clone)]
pub enum Message {
    /// An event from the core actor.
    Core(CoreEvent),
    /// A discovery snapshot/error.
    Discovery(DiscoveryEvent),
    /// Periodic UI tick (relative times, etc).
    Tick,
    /// Drops notices whose timeout has elapsed.
    PruneNotices,
    /// The OS reported a colour-scheme change.
    SystemTheme(iced::theme::Mode),

    // settings navigation
    SettingsOpen(SettingsPage),
    SettingsBack,

    // config / channel editors
    OpenConfigEditor(Section),
    OpenChannelEditor(i32),
    CloseEditor,
    EditorFieldChanged {
        key: String,
        value: String,
    },
    RandomizeChannelPsk,
    SaveEditor,

    SelectTab(Tab),

    // map view
    MapViewChanged(MapView),
    MapFit,
    /// The map canvas reported its size; used to fetch the right tiles.
    MapBounds(Size),
    /// Enable or disable the online OpenStreetMap tile layer.
    ToggleOnlineTiles(bool),
    TileLoaded(TileKey, iced::widget::image::Handle),
    TileFailed(TileKey),

    // connect view
    StartScan,
    StopScan,
    ManualAddressChanged(String),
    ConnectManual,
    ConnectTo(DeviceAddress),
    ConnectTabSelected(ConnectTab),
    DeviceSearchChanged(String),
    DisconnectPressed,
    ResyncPressed,

    // messages view
    SelectChannel(u32),
    SelectPeer(u32),
    ComposeChanged(String),
    SendPressed,

    // nodes view
    NodeSearchChanged(String),
    NodeSelected(u32),
    NodeDeselected,
    OpenDirectMessage(u32),
    /// Jump to the Nodes tab with this node selected.
    OpenNodeDetails(u32),
    ToggleFavorite(u32),
    ToggleIgnored(u32),
    RequestPosition(u32),
    Traceroute(u32),
    RemoveNode(u32),

    // contacts
    /// Show a node's shareable contact QR code.
    ShareContact(u32),
    CloseContactShare,
    /// Open the paste-a-link import dialog.
    OpenContactImport,
    ContactImportChanged(String),
    SubmitContactImport,
    CloseContactImport,

    // ble pairing
    BlePasskeyChanged(String),
    SubmitBlePasskey,
    DismissBlePairing,

    // settings view
    ThemeChanged(ThemePref),
    ToggleNotifications(bool),
    ToggleAutoConnect(bool),
    ToggleSendOnEnter(bool),
    ToggleScanOnStart(bool),
    ToggleImperial(bool),

    // host integration
    /// Send this computer's timezone to the device (`DeviceConfig.tzdef`).
    FillTimezoneFromHost,
    /// Send this computer's clock to the device.
    SyncClockFromHost,
    /// Ask GeoClue for this computer's location and use it as the device's
    /// fixed position.
    UseHostLocation,
    /// The outcome of [`Message::UseHostLocation`].
    HostLocationReady(Result<crate::location::Fix, String>),
    /// Allow the IP-based location fallback.
    ToggleIpLocation(bool),
    /// Manual fixed position, in decimal degrees.
    ManualLatChanged(String),
    ManualLonChanged(String),
    SetManualPosition,

    // tray / window
    /// An event raised by the system tray.
    Tray(TrayEvent),
    /// The main window id, resolved once at startup.
    WindowId(Option<window::Id>),
    /// The window manager asked to close a window.
    CloseRequested(window::Id),
    /// Hide to the tray instead of quitting when the window is closed.
    ToggleCloseToTray(bool),

    // accessibility
    /// Escape: close the topmost dialog, or clear the selection.
    EscapePressed,
    /// Focus the node search box.
    FocusSearch,
    /// Focus the message composer.
    FocusCompose,
    /// Move keyboard focus to the next focusable widget.
    FocusNext,
    /// Move keyboard focus to the previous focusable widget.
    FocusPrevious,
    /// Boost contrast of borders and secondary text.
    ToggleHighContrast(bool),
    /// Update the in-progress interface scale while the slider is dragged.
    UiScalePreview(f32),
    /// Commit the dragged interface scale and persist it.
    UiScaleCommitted,
    /// Quit the application.
    Quit,
    OwnerLongChanged(String),
    OwnerShortChanged(String),
    SaveOwner,
    RebootPressed,
    ShutdownPressed,
    FactoryResetPressed,

    // chrome
    ToggleLogs,
    ClearLogs,
    DismissNotice(usize),
    /// Copy a string to the system clipboard.
    CopyText(String),
    /// Open a link from a message in the system browser.
    OpenLink(String),
}

/// The application.
pub struct App {
    pub bridge: Bridge,
    pub settings: AppSettings,

    pub tab: Tab,
    pub conn: ConnectionState,
    pub my_node_num: Option<u32>,
    pub my_info: Option<MyNodeInfo>,
    pub metadata: Option<DeviceMetadata>,
    pub nodes: HashMap<u32, NodeInfo>,
    /// Latest observed RSSI per node (the protobuf `NodeInfo` lacks it).
    pub node_rssi: HashMap<u32, i32>,
    pub channels: Vec<Channel>,
    pub device_configs: Vec<Config>,
    pub module_configs: Vec<ModuleConfig>,
    pub latest_telemetry: HashMap<u32, Telemetry>,
    pub messages: Vec<MessageRecord>,
    /// Parsed Markdown for each message, kept here so the view can borrow it.
    pub markdown: HashMap<i64, markdown::Content>,
    pub traceroute: Option<TracerouteInfo>,
    pub editor: Option<Editor>,
    pub channel_editor: Option<ChannelEditor>,
    pub settings_page: SettingsPage,

    pub logs: VecDeque<String>,
    pub notices: VecDeque<Notice>,
    pub show_logs: bool,

    // discovery / connect view
    pub devices: Vec<DiscoveredDevice>,
    pub manual_address: String,
    pub connect_tab: ConnectTab,
    pub device_search: String,
    pub ble_scanning: bool,

    // messages view
    pub conversation: Conversation,
    pub compose: String,

    // nodes view
    pub node_search: String,
    pub selected_node: Option<u32>,
    /// The open contact QR dialog, if any.
    pub contact_qr: Option<ContactQr>,
    /// The open contact import dialog, if any.
    pub contact_import: Option<ContactImport>,
    /// The open BLE pairing dialog, if any.
    pub ble_pairing: Option<BlePairing>,
    /// Set when opening the pairing dialog so the passkey field is focused.
    pub focus_pairing: bool,

    // map view
    pub map_view: Option<MapView>,
    /// Whether the optional online OpenStreetMap tile layer is enabled.
    pub online_tiles: bool,
    /// Fetched map tiles, kept in memory for the session.
    pub map_tiles: HashMap<TileKey, iced::widget::image::Handle>,
    /// Tiles currently being fetched.
    pub map_tiles_inflight: HashSet<TileKey>,
    /// Tiles that failed to fetch; not retried until the layer restarts.
    pub map_tile_errors: HashSet<TileKey>,
    /// Insertion order of cached tiles, used to evict the oldest.
    map_tile_order: VecDeque<TileKey>,
    /// The last canvas size the map reported, used to fit before it renders.
    pub map_size: Size,
    /// Shared HTTP client backing the tile layer.
    http: reqwest::Client,
    /// Caps how many tile requests run at once.
    tile_permits: Arc<tokio::sync::Semaphore>,

    // settings view
    pub owner_long: String,
    pub owner_short: String,
    /// In-progress interface scale while the slider is being dragged.
    pub ui_scale_draft: f32,
    /// Manual fixed position input, in decimal degrees.
    pub manual_lat: String,
    pub manual_lon: String,

    pub now: i64,
    pub system_mode: Option<iced::theme::Mode>,

    // tray / window
    /// The system tray, if it could be started.
    pub tray: TrayHandle,
    tray_state: TrayState,
    /// The main window's id, or `None` while it is hidden in the tray.
    window_id: Option<window::Id>,
}

impl App {
    pub fn new(
        core: CoreHandle,
        discovery: DiscoveryHandle,
        settings: AppSettings,
    ) -> (Self, Task<Message>) {
        // Land on Connect unless we are going to auto-connect to a known device.
        let tab = if settings.auto_connect && settings.last_address.is_some() {
            Tab::Messages
        } else {
            Tab::Connect
        };
        let online_tiles = settings.online_tiles;
        let ui_scale_draft = settings.ui_scale;
        // A daemon starts with no window; open the main one here.
        let (window_id, open_window) = iced::window::open(crate::window_settings());
        let app = Self {
            bridge: Bridge::new(core, discovery),
            settings,
            tab,
            conn: ConnectionState::Disconnected,
            my_node_num: None,
            my_info: None,
            metadata: None,
            nodes: HashMap::new(),
            node_rssi: HashMap::new(),
            channels: Vec::new(),
            device_configs: Vec::new(),
            module_configs: Vec::new(),
            latest_telemetry: HashMap::new(),
            messages: Vec::new(),
            markdown: HashMap::new(),
            traceroute: None,
            editor: None,
            channel_editor: None,
            settings_page: SettingsPage::Hub,
            logs: VecDeque::new(),
            notices: VecDeque::new(),
            show_logs: false,
            devices: Vec::new(),
            manual_address: String::new(),
            connect_tab: ConnectTab::All,
            device_search: String::new(),
            ble_scanning: false,
            conversation: Conversation::Channel(0),
            compose: String::new(),
            node_search: String::new(),
            selected_node: None,
            contact_qr: None,
            contact_import: None,
            ble_pairing: None,
            focus_pairing: false,
            map_view: None,
            online_tiles,
            map_tiles: HashMap::new(),
            map_tiles_inflight: HashSet::new(),
            map_tile_errors: HashSet::new(),
            map_tile_order: VecDeque::new(),
            map_size: Size::new(1000.0, 700.0),
            http: build_http_client(),
            tile_permits: Arc::new(tokio::sync::Semaphore::new(tiles::MAX_CONCURRENT_REQUESTS)),
            owner_long: String::new(),
            owner_short: String::new(),
            ui_scale_draft,
            manual_lat: String::new(),
            manual_lon: String::new(),
            now: mt_persistence::now_unix(),
            system_mode: None,
            tray: TrayHandle::spawn(),
            tray_state: TrayState::default(),
            window_id: Some(window_id),
        };
        let boot = Task::batch([
            iced::system::theme().map(Message::SystemTheme),
            open_window.map(|id| Message::WindowId(Some(id))),
        ]);
        (app, boot)
    }

    /// Dispatch an initial auto-connect if the user asked for one.
    pub fn auto_connect(&mut self) {
        if !self.settings.auto_connect {
            return;
        }
        let Some(address) = self.settings.last_address.clone() else {
            return;
        };
        if let Ok(address) = address.parse::<DeviceAddress>() {
            let _ = self
                .bridge
                .core()
                .try_dispatch(CoreCommand::Connect(address));
        }
    }

    /// Whether the effective theme is light.
    pub fn is_light(&self) -> bool {
        match self.settings.theme {
            ThemePref::Dark => false,
            ThemePref::Light => true,
            ThemePref::System => self.system_mode == Some(iced::theme::Mode::Light),
        }
    }

    /// The interface scale factor, clamped to a usable range.
    pub fn ui_scale(&self) -> f32 {
        self.settings.ui_scale.clamp(0.75, 2.0)
    }

    /// The palette currently in effect.
    pub fn theme(&self) -> Theme {
        if self.is_light() {
            theme::light_theme()
        } else {
            theme::app_theme()
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_connected()
    }

    /// Display name for a node number, resolving from the node map.
    pub fn node_name(&self, num: u32) -> String {
        match self.nodes.get(&num) {
            Some(node) => format::node_name(node),
            None => format::node_id(num),
        }
    }

    pub fn node(&self, num: u32) -> Option<&NodeInfo> {
        self.nodes.get(&num)
    }

    /// Enabled channels (primary or secondary), in index order. Disabled
    /// slots are omitted, matching the official clients.
    pub fn active_channels(&self) -> Vec<&Channel> {
        let mut channels: Vec<&Channel> = self
            .channels
            .iter()
            .filter(|c| c.role != meshtastic_protobufs::meshtastic::channel::Role::Disabled as i32)
            .collect();
        channels.sort_by_key(|c| c.index);
        channels
    }

    /// The LoRa modem preset the node is using, if a radio config has arrived.
    pub fn modem_preset(&self) -> Option<i32> {
        use meshtastic_protobufs::meshtastic::config;
        self.device_configs
            .iter()
            .find_map(|c| match &c.payload_variant {
                Some(config::PayloadVariant::Lora(lora)) => Some(lora.modem_preset),
                _ => None,
            })
    }

    /// Direct-message partners, derived from the message history.
    pub fn peers(&self) -> Vec<u32> {
        let Some(me) = self.my_node_num else {
            return Vec::new();
        };
        let broadcast = u32::MAX;
        let mut peers: Vec<u32> = Vec::new();
        for msg in &self.messages {
            let other = if msg.from == me {
                msg.to
            } else if msg.to == me {
                msg.from
            } else {
                continue;
            };
            if other != broadcast && other != 0 && other != me && !peers.contains(&other) {
                peers.push(other);
            }
        }
        peers
    }

    /// Messages for the active conversation, oldest first.
    pub fn conversation_messages(&self) -> Vec<&MessageRecord> {
        self.messages_for(self.conversation)
    }

    /// Messages belonging to a specific conversation, oldest first.
    pub fn messages_for(&self, conversation: Conversation) -> Vec<&MessageRecord> {
        let Some(me) = self.my_node_num else {
            return Vec::new();
        };
        let broadcast = u32::MAX;
        match conversation {
            Conversation::Channel(channel) => self
                .messages
                .iter()
                .filter(|m| m.channel == channel && m.to == broadcast)
                .collect(),
            Conversation::Peer(peer) => self
                .messages
                .iter()
                .filter(|m| (m.from == me && m.to == peer) || (m.from == peer && m.to == me))
                .collect(),
        }
    }

    /// The newest message in a conversation, if any.
    pub fn latest_message(&self, conversation: Conversation) -> Option<&MessageRecord> {
        self.messages_for(conversation).into_iter().last()
    }

    /// Nodes matching the current search, sorted favourites-first then name.
    /// Discovered devices matching the active tab and search box.
    pub fn filtered_devices(&self) -> Vec<&DiscoveredDevice> {
        let needle = self.device_search.trim().to_lowercase();
        self.devices
            .iter()
            .filter(|device| {
                let kind_ok = match self.connect_tab {
                    ConnectTab::All => true,
                    ConnectTab::Serial => device.address.kind() == TransportKind::Serial,
                    ConnectTab::Bluetooth => device.address.kind() == TransportKind::Ble,
                    ConnectTab::Ip => device.address.kind() == TransportKind::Tcp,
                };
                let search_ok = needle.is_empty()
                    || device.name.to_lowercase().contains(&needle)
                    || device.address.to_string().to_lowercase().contains(&needle);
                kind_ok && search_ok
            })
            .collect()
    }

    /// How many discovered devices fall under `tab` (ignoring the search box).
    pub fn device_count(&self, tab: ConnectTab) -> usize {
        self.devices
            .iter()
            .filter(|device| match tab {
                ConnectTab::All => true,
                ConnectTab::Serial => device.address.kind() == TransportKind::Serial,
                ConnectTab::Bluetooth => device.address.kind() == TransportKind::Ble,
                ConnectTab::Ip => device.address.kind() == TransportKind::Tcp,
            })
            .count()
    }

    pub fn visible_nodes(&self) -> Vec<&NodeInfo> {
        let needle = self.node_search.trim().to_lowercase();
        let mut nodes: Vec<&NodeInfo> = self
            .nodes
            .values()
            .filter(|node| {
                if needle.is_empty() {
                    return true;
                }
                let name = format::node_name(node).to_lowercase();
                name.contains(&needle)
                    || format::node_id(node.num).contains(&needle)
                    || format::node_short_name(node)
                        .to_lowercase()
                        .contains(&needle)
            })
            .collect();
        nodes.sort_by(|a, b| {
            b.is_favorite.cmp(&a.is_favorite).then_with(|| {
                format::node_name(a)
                    .to_lowercase()
                    .cmp(&format::node_name(b).to_lowercase())
            })
        });
        nodes
    }

    /// Push a transient notice for the chrome footer.
    pub fn push_notice(&mut self, text: impl Into<String>) {
        self.notices.push_back(Notice {
            text: text.into(),
            expires: Instant::now() + NOTICE_TTL,
        });
        while self.notices.len() > 5 {
            self.notices.pop_front();
        }
    }

    /// Drop notices whose time has run out.
    fn prune_notices(&mut self) {
        let now = Instant::now();
        self.notices.retain(|notice| notice.expires > now);
    }

    // map

    /// Nodes that report a position, local first, then alphabetically. This is
    /// the single source used by both the map view and the tile planner.
    pub fn map_nodes(&self) -> Vec<MapNode> {
        let me = self.my_node_num;
        let mut nodes: Vec<MapNode> = self
            .nodes
            .values()
            .filter_map(|node| {
                let (lat, lon) = position_coords(node.position.as_ref()?)?;
                Some(MapNode {
                    num: node.num,
                    name: format::node_name(node),
                    short: format::node_short_name(node),
                    lat,
                    lon,
                    is_me: me == Some(node.num),
                })
            })
            .collect();
        nodes.sort_by(|a, b| {
            b.is_me
                .cmp(&a.is_me)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        nodes
    }

    /// The local node's coordinates, if it has reported a position.
    pub fn my_coords(&self) -> Option<(f64, f64)> {
        let me = self.my_node_num?;
        position_coords(self.nodes.get(&me)?.position.as_ref()?)
    }

    /// Fetch any online tiles missing for the current map viewport.
    ///
    /// Returns a batch of fetch tasks, or nothing when the layer is off or the
    /// map is not showing. Tiles already cached, in flight, or that previously
    /// failed are skipped, so this is safe to call on every tick.
    fn ensure_tiles(&mut self) -> Task<Message> {
        if !self.online_tiles || self.tab != Tab::Map {
            return Task::none();
        }

        let nodes = self.map_nodes();
        let view = self
            .map_view
            .unwrap_or_else(|| MapView::fit(&nodes, self.map_size));

        let mut tasks = Vec::new();
        for (key, _) in tiles::visible_tiles(&view, self.map_size, tiles::MAX_VISIBLE_TILES) {
            if self.map_tiles.contains_key(&key)
                || self.map_tiles_inflight.contains(&key)
                || self.map_tile_errors.contains(&key)
            {
                continue;
            }
            self.map_tiles_inflight.insert(key);
            let client = self.http.clone();
            let permits = self.tile_permits.clone();
            tasks.push(Task::perform(
                tiles::fetch(client, permits, key),
                |update| match update {
                    tiles::TileUpdate::Loaded(key, handle) => Message::TileLoaded(key, handle),
                    tiles::TileUpdate::Failed(key) => Message::TileFailed(key),
                },
            ));
        }
        Task::batch(tasks)
    }

    /// Parse the manual position fields and send them as the fixed position.
    fn set_manual_position(&mut self) {
        let latitude: f64 = match self.manual_lat.trim().parse() {
            Ok(value) => value,
            Err(_) => {
                self.push_notice("latitude must be a decimal number");
                return;
            }
        };
        let longitude: f64 = match self.manual_lon.trim().parse() {
            Ok(value) => value,
            Err(_) => {
                self.push_notice("longitude must be a decimal number");
                return;
            }
        };
        if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
            self.push_notice("coordinates out of range (lat -90..90, lon -180..180)");
            return;
        }
        let position = Position {
            latitude_i: Some((latitude * 1e7) as i32),
            longitude_i: Some((longitude * 1e7) as i32),
            time: mt_persistence::now_unix() as u32,
            ..Default::default()
        };
        let _ = self
            .bridge
            .core()
            .try_dispatch(CoreCommand::SetFixedPosition(position));
        self.push_notice(format!(
            "manual position sent: {latitude:.5}, {longitude:.5}"
        ));
    }

    /// Close the topmost dismissible UI, or clear the node selection.
    fn escape_pressed(&mut self) {
        if self.contact_import.is_some() {
            self.contact_import = None;
        } else if self.contact_qr.is_some() {
            self.contact_qr = None;
        } else if self.editor.is_some() || self.channel_editor.is_some() {
            self.editor = None;
            self.channel_editor = None;
        } else if self.selected_node.is_some() {
            self.selected_node = None;
        }
    }

    // tray

    /// Handle an event raised by the system tray.
    fn handle_tray(&mut self, event: TrayEvent) -> Task<Message> {
        match event {
            TrayEvent::Toggle => self.toggle_window(),
            TrayEvent::Quit => iced::exit(),
        }
    }

    /// Show or hide the main window, as asked by the tray.
    ///
    /// The app runs as an iced daemon, so hiding is simply closing the window
    /// (the process stays alive) and showing is opening a fresh one. This is
    /// the only approach that works on Wayland, where a window cannot be
    /// hidden or un-minimized programmatically.
    fn toggle_window(&mut self) -> Task<Message> {
        match self.window_id {
            Some(id) => {
                self.window_id = None;
                iced::window::close(id)
            }
            None => self.open_window(),
        }
    }

    /// Open (or reopen) the main window and remember its id.
    fn open_window(&mut self) -> Task<Message> {
        let (id, open) = iced::window::open(crate::window_settings());
        self.window_id = Some(id);
        open.map(|id| Message::WindowId(Some(id)))
    }

    /// Push the connection and node status to the tray, if it changed.
    fn sync_tray(&mut self) {
        let connected = self.is_connected();
        let status = if connected {
            "Connected"
        } else {
            "Disconnected"
        }
        .to_string();

        let mut parts = Vec::new();
        if connected && !self.owner_short.is_empty() {
            parts.push(self.owner_short.clone());
        }
        if !self.nodes.is_empty() {
            parts.push(format!("{} nodes", self.nodes.len()));
        }
        if connected {
            parts.push(format!("{} channels", self.active_channels().len()));
            if let Some(firmware) = self
                .metadata
                .as_ref()
                .map(|metadata| metadata.firmware_version.as_str())
                .filter(|firmware| !firmware.is_empty())
            {
                parts.push(format!("fw {firmware}"));
            }
        }

        let state = TrayState {
            status,
            stats: parts.join(" · "),
        };
        if state != self.tray_state {
            self.tray_state = state.clone();
            self.tray.update(state);
        }
    }

    // update

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Core(event) => {
                self.handle_core_event(event);
                self.sync_tray();
            }
            Message::Discovery(event) => self.handle_discovery_event(event),
            Message::Tick => {
                self.now = mt_persistence::now_unix();
                self.sync_tray();
                // Keep the online tile layer topped up as the map moves or the
                // local node reports a new position.
                return self.ensure_tiles();
            }
            Message::PruneNotices => self.prune_notices(),
            Message::SystemTheme(mode) => self.system_mode = Some(mode),

            Message::SelectTab(tab) => {
                self.tab = tab;
                if tab == Tab::Settings {
                    self.settings_page = SettingsPage::Hub;
                }
                if tab == Tab::Connect && self.settings.scan_ble_on_start && !self.ble_scanning {
                    self.start_scan();
                }
                if tab == Tab::Map {
                    return self.ensure_tiles();
                }
            }
            Message::MapViewChanged(view) => {
                self.map_view = Some(view);
                return self.ensure_tiles();
            }
            Message::MapFit => {
                self.map_view = None;
                return self.ensure_tiles();
            }
            Message::MapBounds(size) => {
                self.map_size = size;
                return self.ensure_tiles();
            }
            Message::ToggleOnlineTiles(enabled) => {
                self.online_tiles = enabled;
                self.settings.online_tiles = enabled;
                self.settings.save();
                if enabled {
                    self.map_tile_errors.clear();
                    return self.ensure_tiles();
                }
            }
            Message::TileLoaded(key, handle) => {
                self.map_tiles_inflight.remove(&key);
                self.map_tile_errors.remove(&key);
                self.map_tiles.insert(key, handle);
                self.map_tile_order.push_back(key);
                while self.map_tile_order.len() > tiles::MAX_CACHED_TILES {
                    match self.map_tile_order.pop_front() {
                        Some(oldest) if oldest != key => {
                            self.map_tiles.remove(&oldest);
                        }
                        Some(_) => break,
                        None => break,
                    }
                }
            }
            Message::TileFailed(key) => {
                self.map_tiles_inflight.remove(&key);
                self.map_tile_errors.insert(key);
            }
            Message::SettingsOpen(page) => {
                self.settings_page = page;
                self.editor = None;
                self.channel_editor = None;
            }
            Message::SettingsBack => {
                self.editor = None;
                self.channel_editor = None;
                self.settings_page = SettingsPage::Hub;
            }

            Message::StartScan => self.start_scan(),
            Message::StopScan => self.stop_scan(),
            Message::ManualAddressChanged(value) => self.manual_address = value,
            Message::ConnectManual => {
                let text = self.manual_address.trim();
                if text.is_empty() {
                    return Task::none();
                }
                match DeviceAddress::parse_manual(text) {
                    Ok(address) => self.connect(address),
                    Err(_) => self.push_notice(format!(
                        "`{text}` is not a valid address (try 192.168.1.42, /dev/ttyUSB0, or a BLE MAC)"
                    )),
                }
            }
            Message::ConnectTo(address) => self.connect(address),
            Message::ConnectTabSelected(tab) => self.connect_tab = tab,
            Message::DeviceSearchChanged(value) => self.device_search = value,
            Message::DisconnectPressed => {
                let _ = self.bridge.core().try_dispatch(CoreCommand::Disconnect);
            }
            Message::ResyncPressed => {
                let _ = self.bridge.core().try_dispatch(CoreCommand::Resync);
            }

            Message::SelectChannel(index) => {
                self.conversation = Conversation::Channel(index);
            }
            Message::SelectPeer(peer) => {
                self.conversation = Conversation::Peer(peer);
            }
            Message::ComposeChanged(value) => self.compose = value,
            Message::SendPressed => self.send_message(),

            Message::NodeSearchChanged(value) => self.node_search = value,
            Message::NodeSelected(num) => self.selected_node = Some(num),
            Message::NodeDeselected => self.selected_node = None,
            Message::OpenDirectMessage(num) => {
                self.conversation = Conversation::Peer(num);
                self.tab = Tab::Messages;
            }
            Message::OpenNodeDetails(num) => {
                self.selected_node = Some(num);
                self.tab = Tab::Nodes;
            }
            Message::ToggleFavorite(num) => {
                let favorite = !self.nodes.get(&num).map(|n| n.is_favorite).unwrap_or(false);
                if let Some(node) = self.nodes.get_mut(&num) {
                    node.is_favorite = favorite;
                }
                let _ = self.bridge.core().try_dispatch(CoreCommand::SetFavorite {
                    node_num: num,
                    favorite,
                });
            }
            Message::ToggleIgnored(num) => {
                let ignored = !self.nodes.get(&num).map(|n| n.is_ignored).unwrap_or(false);
                if let Some(node) = self.nodes.get_mut(&num) {
                    node.is_ignored = ignored;
                }
                let _ = self.bridge.core().try_dispatch(CoreCommand::SetIgnored {
                    node_num: num,
                    ignored,
                });
            }
            Message::RequestPosition(num) => {
                let _ = self
                    .bridge
                    .core()
                    .try_dispatch(CoreCommand::RequestPosition(num));
                self.push_notice(format!("requested position from {}", self.node_name(num)));
            }
            Message::Traceroute(num) => {
                let _ = self
                    .bridge
                    .core()
                    .try_dispatch(CoreCommand::Traceroute(num));
                self.push_notice(format!("traceroute to {} started", self.node_name(num)));
            }
            Message::RemoveNode(num) => {
                let _ = self
                    .bridge
                    .core()
                    .try_dispatch(CoreCommand::RemoveNode(num));
                self.nodes.remove(&num);
                if self.selected_node == Some(num) {
                    self.selected_node = None;
                }
            }

            Message::ShareContact(num) => self.share_contact(num),
            Message::CloseContactShare => self.contact_qr = None,
            Message::OpenContactImport => self.contact_import = Some(ContactImport::default()),
            Message::ContactImportChanged(value) => self.contact_import_changed(value),
            Message::SubmitContactImport => self.submit_contact_import(),
            Message::CloseContactImport => self.contact_import = None,

            Message::BlePasskeyChanged(value) => {
                if let Some(pairing) = self.ble_pairing.as_mut() {
                    // Keep it numeric and at most six digits.
                    pairing.input = value
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .take(6)
                        .collect();
                }
                // Submit as soon as the sixth digit lands: the device only
                // allows about twenty seconds to enter the code.
                if self
                    .ble_pairing
                    .as_ref()
                    .is_some_and(|pairing| pairing.input.len() == 6)
                {
                    let _ = self.submit_ble_passkey();
                }
            }
            Message::SubmitBlePasskey => {
                let _ = self.submit_ble_passkey();
            }
            Message::DismissBlePairing => {
                self.ble_pairing = None;
                // Abort the pairing attempt by dropping the connection.
                let _ = self.bridge.core().try_dispatch(CoreCommand::Disconnect);
            }

            Message::ThemeChanged(pref) => {
                self.settings.theme = pref;
                self.settings.save();
            }
            Message::ToggleNotifications(value) => {
                self.settings.notifications = value;
                self.settings.save();
            }
            Message::ToggleAutoConnect(value) => {
                self.settings.auto_connect = value;
                self.settings.save();
                let _ = self
                    .bridge
                    .core()
                    .try_dispatch(CoreCommand::SetAutoReconnect(value));
            }
            Message::ToggleSendOnEnter(value) => {
                self.settings.send_on_enter = value;
                self.settings.save();
            }
            Message::ToggleScanOnStart(value) => {
                self.settings.scan_ble_on_start = value;
                self.settings.save();
            }
            Message::ToggleImperial(value) => {
                self.settings.imperial = value;
                self.settings.save();
            }

            Message::FillTimezoneFromHost => {
                match crate::host::posix_tzdef() {
                    Some(tzdef) => {
                        // Preserve every other device field and only replace tzdef.
                        let mut device = self
                        .device_configs
                        .iter()
                        .find_map(|config| match &config.payload_variant {
                            Some(
                                meshtastic_protobufs::meshtastic::config::PayloadVariant::Device(device),
                            ) => Some(device.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                        device.tzdef = tzdef.clone();
                        self.apply_config_value(SectionValue::Device(device));
                        self.push_notice(format!("timezone sent from host: {tzdef}"));
                    }
                    None => self.push_notice("could not determine the host timezone"),
                }
            }
            Message::SyncClockFromHost => {
                let seconds = mt_persistence::now_unix() as u32;
                let _ = self
                    .bridge
                    .core()
                    .try_dispatch(CoreCommand::SetTime(seconds));
                self.push_notice("device clock set from host");
            }
            Message::UseHostLocation => {
                self.push_notice("asking the host for a location");
                let client = self.http.clone();
                let allow_ip = self.settings.use_ip_location;
                return Task::perform(
                    crate::location::locate(client, allow_ip),
                    Message::HostLocationReady,
                );
            }
            Message::HostLocationReady(Ok(fix)) => {
                let position = Position {
                    latitude_i: Some((fix.latitude * 1e7) as i32),
                    longitude_i: Some((fix.longitude * 1e7) as i32),
                    time: mt_persistence::now_unix() as u32,
                    ..Default::default()
                };
                let _ = self
                    .bridge
                    .core()
                    .try_dispatch(CoreCommand::SetFixedPosition(position));
                let fix_note = if fix.accuracy > 0.0 {
                    format!(
                        "{} location sent: {:.5}, {:.5} (±{:.0} m)",
                        fix.source, fix.latitude, fix.longitude, fix.accuracy
                    )
                } else {
                    format!(
                        "{} location sent: {:.5}, {:.5}",
                        fix.source, fix.latitude, fix.longitude
                    )
                };
                self.push_notice(fix_note);
            }
            Message::HostLocationReady(Err(error)) => {
                self.push_notice(format!("host location failed: {error}"));
            }
            Message::ToggleIpLocation(value) => {
                self.settings.use_ip_location = value;
                self.settings.save();
            }
            Message::ManualLatChanged(value) => self.manual_lat = value,
            Message::ManualLonChanged(value) => self.manual_lon = value,
            Message::SetManualPosition => self.set_manual_position(),

            Message::Tray(event) => return self.handle_tray(event),
            Message::WindowId(id) => self.window_id = id,
            Message::CloseRequested(id) => {
                if self.window_id == Some(id) {
                    self.window_id = None;
                }
                if self.settings.close_to_tray {
                    return iced::window::close(id);
                }
                return iced::exit();
            }
            Message::ToggleCloseToTray(value) => {
                self.settings.close_to_tray = value;
                self.settings.save();
            }

            Message::EscapePressed => self.escape_pressed(),
            Message::FocusSearch => {
                self.tab = Tab::Nodes;
                return iced::widget::operation::focus(crate::views::nodes::SEARCH_INPUT_ID);
            }
            Message::FocusCompose => {
                self.tab = Tab::Messages;
                return iced::widget::operation::focus(crate::views::messages::COMPOSE_INPUT_ID);
            }
            Message::FocusNext => return iced::widget::operation::focus_next(),
            Message::FocusPrevious => return iced::widget::operation::focus_previous(),
            Message::ToggleHighContrast(value) => {
                self.settings.high_contrast = value;
                self.settings.save();
            }
            Message::UiScalePreview(value) => self.ui_scale_draft = value,
            Message::UiScaleCommitted => {
                self.settings.ui_scale = self.ui_scale_draft;
                self.settings.save();
            }
            Message::Quit => return iced::exit(),
            Message::OwnerLongChanged(value) => self.owner_long = value,
            Message::OwnerShortChanged(value) => self.owner_short = value,
            Message::SaveOwner => {
                let _ = self.bridge.core().try_dispatch(CoreCommand::SetOwner {
                    long_name: self.owner_long.trim().to_string(),
                    short_name: self.owner_short.trim().to_string(),
                    is_licensed: false,
                });
                self.push_notice("owner update sent");
            }
            Message::RebootPressed => {
                let _ = self.bridge.core().try_dispatch(CoreCommand::Reboot {
                    dest: u32::MAX,
                    seconds: 2,
                });
                self.push_notice("reboot requested");
            }
            Message::ShutdownPressed => {
                let _ = self.bridge.core().try_dispatch(CoreCommand::Shutdown {
                    dest: u32::MAX,
                    seconds: 2,
                });
                self.push_notice("shutdown requested");
            }
            Message::FactoryResetPressed => {
                let _ = self.bridge.core().try_dispatch(CoreCommand::FactoryReset {
                    dest: u32::MAX,
                    full_device: false,
                });
                self.push_notice("factory reset (config) requested");
            }

            Message::ToggleLogs => self.show_logs = !self.show_logs,
            Message::ClearLogs => self.logs.clear(),
            Message::OpenConfigEditor(section) => {
                let current = if section.is_module() {
                    self.module_configs
                        .iter()
                        .find_map(|c| SectionValue::from_module_config(section, c))
                } else {
                    self.device_configs
                        .iter()
                        .find_map(|c| SectionValue::from_config(section, c))
                };
                self.editor = Some(Editor::new(section, current));
                self.channel_editor = None;
                self.settings_page = SettingsPage::Device;
            }
            Message::OpenChannelEditor(index) => {
                let original = self
                    .channels
                    .iter()
                    .find(|c| c.index == index)
                    .cloned()
                    .unwrap_or_else(|| Channel {
                        index,
                        role: if index == 0 {
                            meshtastic_protobufs::meshtastic::channel::Role::Primary as i32
                        } else {
                            meshtastic_protobufs::meshtastic::channel::Role::Disabled as i32
                        },
                        settings: Some(Default::default()),
                    });
                self.channel_editor = Some(ChannelEditor::new(index, original));
                self.editor = None;
                self.settings_page = SettingsPage::Channels;
            }
            Message::CloseEditor => {
                self.editor = None;
                self.channel_editor = None;
            }
            Message::EditorFieldChanged { key, value } => {
                if let Some(editor) = &mut self.editor {
                    editor.set(&key, value);
                } else if let Some(editor) = &mut self.channel_editor {
                    editor.set(&key, value);
                }
            }
            Message::RandomizeChannelPsk => {
                if let Some(editor) = &mut self.channel_editor {
                    editor.set("psk", crate::config_editor::random_channel_key());
                }
            }
            Message::SaveEditor => self.save_editor(),

            Message::DismissNotice(index) => {
                if index < self.notices.len() {
                    self.notices.remove(index);
                }
            }

            Message::CopyText(text) => {
                self.push_notice("copied to clipboard");
                return iced::clipboard::write(text);
            }

            Message::OpenLink(url) => {
                if let Err(err) = open_url(&url) {
                    self.push_notice(format!("could not open link: {err}"));
                }
            }
        }
        if std::mem::take(&mut self.focus_pairing) {
            return iced::widget::operation::focus(crate::views::pairing::PASSKEY_INPUT_ID);
        }
        Task::none()
    }

    fn save_editor(&mut self) {
        // Config section editor.
        if let Some(result) = self.editor.as_ref().map(Editor::build) {
            match result {
                Ok(value) => {
                    self.editor = None;
                    self.apply_config_value(value);
                    self.push_notice("configuration saved to the device");
                }
                Err(error) => {
                    if let Some(editor) = &mut self.editor {
                        editor.error = Some(error);
                    }
                }
            }
        }

        // Channel editor.
        if let Some(result) = self.channel_editor.as_ref().map(ChannelEditor::build) {
            match result {
                Ok(channel) => {
                    self.channel_editor = None;
                    self.apply_channel_value(channel);
                    self.push_notice("channel saved to the device");
                }
                Err(error) => {
                    if let Some(editor) = &mut self.channel_editor {
                        editor.error = Some(error);
                    }
                }
            }
        }
    }

    fn apply_config_value(&mut self, value: SectionValue) {
        if let Some(config) = value.to_config() {
            self.device_configs
                .retain(|c| !same_device_config(c, &config));
            self.device_configs.push(config.clone());
            let _ = self
                .bridge
                .core()
                .try_dispatch(CoreCommand::SetConfig(Box::new(config)));
        } else if let Some(module) = value.to_module_config() {
            self.module_configs
                .retain(|c| !same_module_config(c, &module));
            self.module_configs.push(module.clone());
            let _ = self
                .bridge
                .core()
                .try_dispatch(CoreCommand::SetModuleConfig(Box::new(module)));
        }
    }

    fn apply_channel_value(&mut self, channel: Channel) {
        match self.channels.iter_mut().find(|c| c.index == channel.index) {
            Some(slot) => *slot = channel.clone(),
            None => {
                self.channels.push(channel.clone());
                self.channels.sort_by_key(|c| c.index);
            }
        }
        let _ = self
            .bridge
            .core()
            .try_dispatch(CoreCommand::SetChannel(Box::new(channel)));
    }

    fn connect(&mut self, address: DeviceAddress) {
        self.settings.last_address = Some(address.to_string());
        self.settings.save();
        self.tab = Tab::Messages;
        self.stop_scan();
        let _ = self
            .bridge
            .core()
            .try_dispatch(CoreCommand::Connect(address));
    }

    fn start_scan(&mut self) {
        self.ble_scanning = true;
        self.bridge.discovery().try_start_ble_scan();
        self.push_notice("scanning for Bluetooth devices…");
    }

    fn stop_scan(&mut self) {
        if self.ble_scanning {
            self.ble_scanning = false;
            self.bridge.discovery().try_stop_ble_scan();
        }
    }

    fn send_message(&mut self) {
        let text = self.compose.trim().to_string();
        if text.is_empty() {
            return;
        }
        if text.len() > MAX_MESSAGE_BYTES {
            self.push_notice(format!(
                "message is too long (limit {MAX_MESSAGE_BYTES} bytes)"
            ));
            return;
        }
        let (channel, to) = match self.conversation {
            Conversation::Channel(channel) => (channel, None),
            Conversation::Peer(peer) => (0, Some(peer)),
        };
        let _ = self.bridge.core().try_dispatch(CoreCommand::SendText {
            text,
            channel,
            to,
            reply_id: None,
        });
        self.compose.clear();
    }

    /// Build and open the shareable contact QR for a node.
    fn share_contact(&mut self, num: u32) {
        let Some(node) = self.nodes.get(&num) else {
            return;
        };
        let Some(user) = node.user.clone() else {
            self.push_notice("this node has no user info to share");
            return;
        };
        let Some(contact) = mt_protocol::contact::shared_contact_for(num, &user) else {
            self.push_notice("this node has no public key to share");
            return;
        };
        let uri = mt_protocol::contact::shared_contact_url(&contact);
        match iced::widget::qr_code::Data::new(uri.as_bytes()) {
            Ok(data) => {
                self.contact_qr = Some(ContactQr {
                    title: format!("Share {}", format::node_name(node)),
                    uri,
                    data,
                });
            }
            Err(_) => self.push_notice("this contact is too large to fit in a QR code"),
        }
    }

    /// Re-parse the import dialog input into a preview (and a ready contact).
    fn contact_import_changed(&mut self, value: String) {
        let Some(import) = self.contact_import.as_mut() else {
            return;
        };
        import.input = value;
        import.error = None;
        import.preview = None;
        import.contact = None;

        let text = import.input.trim();
        if text.is_empty() {
            return;
        }
        match mt_protocol::contact::parse_shared_contact(text) {
            Ok(contact) => {
                let Some(user) = contact.user.as_ref() else {
                    import.error = Some("the contact is missing its user info".into());
                    return;
                };
                if contact.node_num == 0 {
                    import.error = Some("the contact is missing its node number".into());
                    return;
                }
                if !mt_protocol::contact::has_public_key(user) {
                    import.error = Some(
                        "the contact has no public key, so importing would clear a stored key"
                            .into(),
                    );
                    return;
                }
                let name = if user.long_name.trim().is_empty() {
                    user.short_name.clone()
                } else {
                    user.long_name.clone()
                };
                import.preview = Some(ContactPreview {
                    name,
                    id: format::node_id(contact.node_num),
                });
                import.contact = Some(contact);
            }
            Err(err) => import.error = Some(err.to_string()),
        }
    }

    /// Send the decoded contact to the device and close the dialog.
    fn submit_contact_import(&mut self) {
        let Some(import) = self.contact_import.as_ref() else {
            return;
        };
        let Some(contact) = import.contact.clone() else {
            return;
        };
        let name = import
            .preview
            .as_ref()
            .map(|preview| preview.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format::node_id(contact.node_num));
        let _ = self
            .bridge
            .core()
            .try_dispatch(CoreCommand::AddContact(Box::new(contact)));
        self.contact_import = None;
        self.push_notice(format!("importing contact {name}"));
    }

    /// Send the BLE pairing passkey the user typed to the transport.
    fn submit_ble_passkey(&mut self) {
        let Some(pairing) = self.ble_pairing.as_ref() else {
            return;
        };
        match pairing.input.trim().parse::<u32>() {
            Ok(passkey) => {
                let _ = self
                    .bridge
                    .core()
                    .try_dispatch(CoreCommand::SubmitBlePasskey(passkey));
                self.ble_pairing = None;
                self.push_notice("pairing passkey sent");
            }
            Err(_) => self.push_notice("enter the 6-digit code shown on the device"),
        }
    }

    fn handle_core_event(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Connection(state) => {
                if matches!(state, ConnectionState::Connecting(_)) {
                    // Fresh connection: drop the previous device's view of
                    // the world so nothing bleeds across devices.
                    self.nodes.clear();
                    self.node_rssi.clear();
                    self.channels.clear();
                    self.device_configs.clear();
                    self.module_configs.clear();
                    self.latest_telemetry.clear();
                    self.messages.clear();
                    self.markdown.clear();
                    self.my_node_num = None;
                    self.my_info = None;
                    self.metadata = None;
                    self.traceroute = None;
                    self.contact_qr = None;
                    self.contact_import = None;
                    self.ble_pairing = None;
                }
                if let ConnectionState::Connected { address, node_num } = &state {
                    self.settings.last_address = Some(address.to_string());
                    self.settings.save();
                    self.my_node_num = Some(*node_num);
                    self.ble_pairing = None;
                    let _ = self.bridge.discovery().stop_ble_scan();
                    tracing::info!(
                        node_num,
                        device_configs = self.device_configs.len(),
                        module_configs = self.module_configs.len(),
                        nodes = self.nodes.len(),
                        channels = self.channels.len(),
                        "handshake complete in ui"
                    );
                }
                self.conn = state;
            }
            CoreEvent::MyInfo(info) => {
                self.my_node_num = Some(info.my_node_num);
                if let Some(node) = self.nodes.get(&info.my_node_num) {
                    self.owner_long = node
                        .user
                        .as_ref()
                        .map(|u| u.long_name.clone())
                        .unwrap_or_default();
                    self.owner_short = node
                        .user
                        .as_ref()
                        .map(|u| u.short_name.clone())
                        .unwrap_or_default();
                }
                self.my_info = Some(*info);
            }
            CoreEvent::Metadata(metadata) => self.metadata = Some(*metadata),
            CoreEvent::Config(config) => {
                let config = *config;
                self.device_configs
                    .retain(|c| !same_device_config(c, &config));
                self.device_configs.push(config);
            }
            CoreEvent::ModuleConfig(config) => {
                let config = *config;
                self.module_configs
                    .retain(|c| !same_module_config(c, &config));
                self.module_configs.push(config);
            }
            CoreEvent::Channel(channel) => {
                let channel = *channel;
                match self.channels.iter_mut().find(|c| c.index == channel.index) {
                    Some(slot) => *slot = channel,
                    None => {
                        self.channels.push(channel);
                        self.channels.sort_by_key(|c| c.index);
                    }
                }
            }
            CoreEvent::Node(node) => {
                let node = *node;
                if node.num == self.my_node_num.unwrap_or(u32::MAX) {
                    if let Some(user) = &node.user {
                        if !user.long_name.is_empty() {
                            self.owner_long = user.long_name.clone();
                        }
                        if !user.short_name.is_empty() {
                            self.owner_short = user.short_name.clone();
                        }
                    }
                }
                self.nodes.insert(node.num, node);
            }
            CoreEvent::NodeRemoved(num) => {
                self.nodes.remove(&num);
            }
            CoreEvent::Rssi { node_num, rssi } => {
                self.node_rssi.insert(node_num, rssi);
            }
            CoreEvent::MessagesLoaded(messages) => {
                self.messages = messages;
                self.sort_messages();
                self.markdown = self
                    .messages
                    .iter()
                    .filter(|message| !message.text.trim().is_empty())
                    .map(|message| {
                        (
                            message.id,
                            markdown::Content::parse(&format::autolink(&message.text)),
                        )
                    })
                    .collect();
            }
            CoreEvent::Message(record) => {
                if !record.outgoing && self.settings.notifications {
                    self.notify_incoming(&record);
                }
                self.merge_message(*record);
            }
            CoreEvent::MessageStatus {
                packet_id,
                outgoing,
                status,
                error,
            } => {
                if let Some(message) = self
                    .messages
                    .iter_mut()
                    .find(|m| m.packet_id == packet_id && m.outgoing == outgoing)
                {
                    message.status = status;
                    message.error = error;
                }
            }
            CoreEvent::Position { node_num, position } => {
                self.apply_position(node_num, &position);
            }
            CoreEvent::Telemetry {
                node_num,
                telemetry,
            } => {
                self.latest_telemetry.insert(node_num, *telemetry);
            }
            CoreEvent::Traceroute {
                from,
                route,
                snr_towards,
                route_back,
                snr_back,
                ..
            } => {
                self.traceroute = Some(TracerouteInfo {
                    from,
                    route,
                    snr_towards,
                    route_back,
                    snr_back,
                    at: mt_persistence::now_unix(),
                });
            }
            CoreEvent::QueueStatus(_) => {}
            CoreEvent::BlePairingRequest { address } => {
                self.ble_pairing = Some(BlePairing {
                    address,
                    input: String::new(),
                });
                self.focus_pairing = true;
            }
            CoreEvent::DeviceLog(line) => {
                let line = format::sanitize_log_line(&line);
                if !line.is_empty() {
                    self.logs.push_back(line);
                    while self.logs.len() > 500 {
                        self.logs.pop_front();
                    }
                }
            }
            CoreEvent::Rebooted { from_dfu } => {
                self.push_notice(if from_dfu {
                    "device rebooted from DFU"
                } else {
                    "device rebooted"
                });
            }
            CoreEvent::Error(message) => self.push_notice(message),
        }
    }

    fn handle_discovery_event(&mut self, event: DiscoveryEvent) {
        match event {
            DiscoveryEvent::DevicesUpdated(devices) => self.devices = devices,
            DiscoveryEvent::Error(message) => self.push_notice(message),
        }
    }

    fn notify_incoming(&self, record: &MessageRecord) {
        let name = self.node_name(record.from);
        let mut notification = notify_rust::Notification::new();
        let body = if record.text.is_empty() {
            format::portnum_label(record.portnum).to_string()
        } else {
            record.text.clone()
        };
        notification
            .appname("Meshtastic Desktop")
            .summary(&name)
            .body(&body)
            .icon(crate::APP_ID)
            .hint(notify_rust::Hint::DesktopEntry(crate::APP_ID.to_string()));
        let _ = notification.show();
    }

    fn apply_position(&mut self, node_num: u32, position: &Position) {
        if let Some(node) = self.nodes.get_mut(&node_num) {
            node.position = Some(position.clone());
            if position.time != 0 {
                node.last_heard = position.time;
            }
        }
    }

    /// Insert or replace a message, keeping the list ordered.
    fn merge_message(&mut self, record: MessageRecord) {
        if let Some(existing) = self
            .messages
            .iter_mut()
            .find(|m| m.packet_id == record.packet_id && m.outgoing == record.outgoing)
        {
            *existing = record.clone();
        } else {
            self.messages.push(record.clone());
            self.sort_messages();
        }
        self.cache_markdown(&record);
    }

    /// Parse a message's text as Markdown (with bare URLs linked) for display.
    fn cache_markdown(&mut self, record: &MessageRecord) {
        if record.text.trim().is_empty() {
            self.markdown.remove(&record.id);
            return;
        }
        let source = format::autolink(&record.text);
        self.markdown
            .insert(record.id, markdown::Content::parse(&source));
    }

    fn sort_messages(&mut self) {
        self.messages.sort_by_key(|m| (m.sent_at, m.id));
    }

    // view / subscription

    /// All subscriptions: the event bridge plus a slow UI tick.
    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            self.bridge.subscription(),
            self.tray.subscription(),
            iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::Tick),
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::window::close_requests().map(Message::CloseRequested),
            iced::event::listen_with(keyboard_shortcuts),
        ];
        // When "send on Enter" is off, the compose box has no submit handler,
        // so listen for Ctrl+Enter ourselves while the Messages tab is open.
        if self.tab == Tab::Messages && !self.settings.send_on_enter {
            subscriptions.push(iced::event::listen_with(send_on_ctrl_enter));
        }
        // While notices are visible, check often enough to expire them.
        if !self.notices.is_empty() {
            subscriptions.push(
                iced::time::every(std::time::Duration::from_millis(500))
                    .map(|_| Message::PruneNotices),
            );
        }
        Subscription::batch(subscriptions)
    }

    /// Render the whole window: navigation rail plus the active section.
    pub fn view(&self) -> Element<'_, Message> {
        // Publish the palette for this frame so view-time colour accessors
        // resolve to the right mode.
        theme::set_light_mode(self.is_light());
        theme::set_high_contrast(self.settings.high_contrast);

        let content = match self.tab {
            Tab::Messages => views::messages::view(self),
            Tab::Nodes => views::nodes::view(self),
            Tab::Map => views::map::view(self),
            Tab::Connect => views::connect::view(self),
            Tab::Settings => views::settings::view(self),
        };

        let mut right = column![content].width(Length::Fill).height(Length::Fill);
        if let Some((index, notice)) = self
            .notices
            .iter()
            .enumerate()
            .next_back()
            .map(|(i, n)| (i, n.text.clone()))
        {
            right = right.push(notice_bar(notice, index));
        }

        let base: Element<'_, Message> = row![views::nav::view(self), right]
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        let dialog = views::contact::overlay(self).or_else(|| views::pairing::overlay(self));
        match dialog {
            Some(dialog) => iced::widget::stack![base, dialog]
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            None => base,
        }
    }
}

/// A dismissible status bar shown at the bottom of the window.
fn notice_bar<'a>(notice: String, index: usize) -> Element<'a, Message> {
    container(
        row![
            crate::icons::lucide::info()
                .size(15)
                .color(theme::warning()),
            text(notice)
                .size(12)
                .color(theme::text())
                .width(Length::Fill),
            button(text("Dismiss").size(11))
                .padding(Padding::from([3, 8]))
                .style(theme::ghost_button)
                .on_press(Message::DismissNotice(index)),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([7, 16]))
    .style(|_: &Theme| iced::widget::container::Style {
        background: Some(theme::surface_alt().into()),
        border: iced::Border {
            color: theme::warning(),
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Whether two device configs are the same section (e.g. both LoRa).
fn same_device_config(a: &Config, b: &Config) -> bool {
    match (a.payload_variant.as_ref(), b.payload_variant.as_ref()) {
        (Some(a), Some(b)) => std::mem::discriminant(a) == std::mem::discriminant(b),
        (None, None) => true,
        _ => false,
    }
}

/// Whether two module configs are the same section (e.g. both MQTT).
fn same_module_config(a: &ModuleConfig, b: &ModuleConfig) -> bool {
    match (a.payload_variant.as_ref(), b.payload_variant.as_ref()) {
        (Some(a), Some(b)) => std::mem::discriminant(a) == std::mem::discriminant(b),
        (None, None) => true,
        _ => false,
    }
}

/// Build the shared HTTP client used by the online map tile layer.
///
/// The OpenStreetMap tile usage policy requires an identifying `User-Agent`.
/// The fallback client is only reached if the builder fails, which is rare.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!(
            "MeshtasticDesktop/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/drakeerv)"
        ))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// The latitude/longitude of a position, if it carries coordinates. Stored as
/// degrees scaled by ten million.
fn position_coords(position: &Position) -> Option<(f64, f64)> {
    Some((
        position.latitude_i? as f64 * 1e-7,
        position.longitude_i? as f64 * 1e-7,
    ))
}

/// Open an external link in the user's browser, rejecting unsafe schemes.
fn open_url(url: &str) -> std::io::Result<()> {
    let allowed =
        url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:");
    if !allowed {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsupported link",
        ));
    }
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

/// Sends the compose buffer on Ctrl+Enter, used when "send on Enter" is off.
fn send_on_ctrl_enter(
    event: iced::event::Event,
    status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    if status != iced::event::Status::Ignored {
        return None;
    }
    if let iced::event::Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Enter),
        modifiers,
        ..
    }) = event
    {
        if modifiers.control() {
            return Some(Message::SendPressed);
        }
    }
    None
}

/// Global keyboard shortcuts.
///
/// The event must be unhandled (`Ignored`) so that typing in a focused text
/// field is never hijacked. These shortcuts are how a keyboard-only user
/// reaches the app's main actions, since iced does not make buttons focusable.
fn keyboard_shortcuts(
    event: iced::event::Event,
    status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    if status != iced::event::Status::Ignored {
        return None;
    }

    let iced::event::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event
    else {
        return None;
    };

    match key {
        keyboard::Key::Named(keyboard::key::Named::Escape) => Some(Message::EscapePressed),
        keyboard::Key::Named(keyboard::key::Named::Tab) => {
            if modifiers.shift() {
                Some(Message::FocusPrevious)
            } else {
                Some(Message::FocusNext)
            }
        }
        keyboard::Key::Character(character) if modifiers.control() => {
            match character.to_lowercase().as_str() {
                "1" => Some(Message::SelectTab(Tab::Messages)),
                "2" => Some(Message::SelectTab(Tab::Nodes)),
                "3" => Some(Message::SelectTab(Tab::Map)),
                "4" => Some(Message::SelectTab(Tab::Connect)),
                "5" => Some(Message::SelectTab(Tab::Settings)),
                "f" => Some(Message::FocusSearch),
                "m" => Some(Message::FocusCompose),
                "r" => Some(Message::ResyncPressed),
                "q" => Some(Message::Quit),
                "," => Some(Message::SelectTab(Tab::Settings)),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    #[tokio::test]
    async fn config_events_populate_editor() {
        let core = mt_core::spawn_core(mt_core::CoreConfig::default());
        let discovery = mt_transport::spawn_discovery();
        let (mut app, _task) = App::new(core, discovery, AppSettings::default());

        let config = Config {
            payload_variant: Some(
                meshtastic_protobufs::meshtastic::config::PayloadVariant::Lora(
                    meshtastic_protobufs::meshtastic::config::LoRaConfig {
                        region: 1,
                        hop_limit: 3,
                        ..Default::default()
                    },
                ),
            ),
        };
        let _ = app.update(Message::Core(CoreEvent::Config(Box::new(config))));
        assert_eq!(app.device_configs.len(), 1, "config stored");

        let _ = app.update(Message::OpenConfigEditor(Section::Lora));
        let editor = app.editor.as_ref().expect("editor opened");
        assert_eq!(editor.value("region"), "1");
        assert_eq!(editor.value("hop_limit"), "3");
    }

    #[tokio::test]
    async fn distinct_config_sections_are_kept() {
        use meshtastic_protobufs::meshtastic::config;
        let core = mt_core::spawn_core(mt_core::CoreConfig::default());
        let discovery = mt_transport::spawn_discovery();
        let (mut app, _task) = App::new(core, discovery, AppSettings::default());

        let lora = |region| Config {
            payload_variant: Some(config::PayloadVariant::Lora(config::LoRaConfig {
                region,
                ..Default::default()
            })),
        };
        let device = Config {
            payload_variant: Some(config::PayloadVariant::Device(config::DeviceConfig {
                node_info_broadcast_secs: 900,
                ..Default::default()
            })),
        };

        let _ = app.update(Message::Core(CoreEvent::Config(Box::new(lora(1)))));
        let _ = app.update(Message::Core(CoreEvent::Config(Box::new(device))));
        let _ = app.update(Message::Core(CoreEvent::Config(Box::new(lora(3)))));
        // Same section updates in place; different sections accumulate.
        assert_eq!(app.device_configs.len(), 2);

        let _ = app.update(Message::OpenConfigEditor(Section::Lora));
        assert_eq!(app.editor.as_ref().unwrap().value("region"), "3");
    }
    #[tokio::test]
    async fn active_channels_hide_disabled_slots() {
        use meshtastic_protobufs::meshtastic::channel::Role;
        let core = mt_core::spawn_core(mt_core::CoreConfig::default());
        let discovery = mt_transport::spawn_discovery();
        let (mut app, _task) = App::new(core, discovery, AppSettings::default());
        app.channels = (0..8)
            .map(|index| Channel {
                index,
                role: match index {
                    0 => Role::Primary as i32,
                    1 => Role::Secondary as i32,
                    _ => Role::Disabled as i32,
                },
                ..Default::default()
            })
            .collect();

        let active: Vec<i32> = app.active_channels().iter().map(|c| c.index).collect();
        assert_eq!(active, vec![0, 1]);
    }
    #[tokio::test]
    async fn notices_are_bounded_and_fresh_ones_survive_pruning() {
        let core = mt_core::spawn_core(mt_core::CoreConfig::default());
        let discovery = mt_transport::spawn_discovery();
        let (mut app, _task) = App::new(core, discovery, AppSettings::default());
        for index in 0..7 {
            app.push_notice(format!("notice {index}"));
        }
        assert_eq!(app.notices.len(), 5);
        // Fresh notices are not pruned immediately.
        app.prune_notices();
        assert_eq!(app.notices.len(), 5);
    }

    #[tokio::test]
    async fn contact_import_parses_valid_links_and_rejects_keyless_ones() {
        use meshtastic_protobufs::meshtastic::{SharedContact, User};

        let core = mt_core::spawn_core(mt_core::CoreConfig::default());
        let discovery = mt_transport::spawn_discovery();
        let (mut app, _task) = App::new(core, discovery, AppSettings::default());
        let _ = app.update(Message::OpenContactImport);

        let contact = |key: Vec<u8>| SharedContact {
            node_num: 0x1234,
            user: Some(User {
                id: "!00001234".into(),
                long_name: "Scout".into(),
                short_name: "SC".into(),
                public_key: key,
                ..Default::default()
            }),
            should_ignore: false,
        };

        // A contact with a key previews and is ready to import.
        let url = mt_protocol::contact::shared_contact_url(&contact(vec![0x22; 32]));
        let _ = app.update(Message::ContactImportChanged(url));
        let import = app.contact_import.as_ref().unwrap();
        assert!(import.error.is_none());
        assert_eq!(
            import.preview.as_ref().map(|preview| preview.name.as_str()),
            Some("Scout")
        );
        assert!(import.contact.is_some());

        // A keyless contact is refused with an explanation.
        let url = mt_protocol::contact::shared_contact_url(&contact(Vec::new()));
        let _ = app.update(Message::ContactImportChanged(url));
        let import = app.contact_import.as_ref().unwrap();
        assert!(import.contact.is_none());
        assert!(import.error.is_some());

        // Submitting a valid import closes the dialog.
        let url = mt_protocol::contact::shared_contact_url(&contact(vec![0x22; 32]));
        let _ = app.update(Message::ContactImportChanged(url));
        let _ = app.update(Message::SubmitContactImport);
        assert!(app.contact_import.is_none());
    }

    #[tokio::test]
    async fn connect_tabs_and_search_filter_devices() {
        let core = mt_core::spawn_core(mt_core::CoreConfig::default());
        let discovery = mt_transport::spawn_discovery();
        let (mut app, _task) = App::new(core, discovery, AppSettings::default());

        let device = |address, name: &str| DiscoveredDevice {
            address,
            name: name.into(),
            detail: String::new(),
            rssi: None,
        };
        app.devices = vec![
            device(DeviceAddress::ble("10:bd:a3:5b:07:f9"), "DH1_07f8"),
            device(DeviceAddress::serial("/dev/ttyACM0"), "/dev/ttyACM0"),
            device(DeviceAddress::tcp("192.168.1.42", 4403), "Base Camp"),
        ];

        assert_eq!(app.device_count(ConnectTab::All), 3);
        assert_eq!(app.device_count(ConnectTab::Serial), 1);
        assert_eq!(app.device_count(ConnectTab::Bluetooth), 1);
        assert_eq!(app.device_count(ConnectTab::Ip), 1);

        app.connect_tab = ConnectTab::Bluetooth;
        assert_eq!(app.filtered_devices().len(), 1);
        assert_eq!(app.filtered_devices()[0].name, "DH1_07f8");

        app.connect_tab = ConnectTab::All;
        app.device_search = "base".into();
        assert_eq!(app.filtered_devices().len(), 1);
        assert_eq!(app.filtered_devices()[0].name, "Base Camp");
    }

    #[tokio::test]
    async fn sharing_a_node_with_a_key_opens_a_qr_dialog() {
        use meshtastic_protobufs::meshtastic::{NodeInfo, User};

        let core = mt_core::spawn_core(mt_core::CoreConfig::default());
        let discovery = mt_transport::spawn_discovery();
        let (mut app, _task) = App::new(core, discovery, AppSettings::default());

        let num = 0x55;
        app.nodes.insert(
            num,
            NodeInfo {
                num,
                user: Some(User {
                    id: "!00000055".into(),
                    long_name: "Relay".into(),
                    public_key: vec![0x33; 32],
                    ..Default::default()
                }),
                ..Default::default()
            },
        );

        let _ = app.update(Message::ShareContact(num));
        let qr = app.contact_qr.as_ref().expect("qr dialog opens");
        assert!(qr.uri.starts_with(mt_protocol::contact::CONTACT_URL_PREFIX));

        let _ = app.update(Message::CloseContactShare);
        assert!(app.contact_qr.is_none());
    }

    /// Build a key-press event for shortcut tests.
    fn press(key: keyboard::Key, modifiers: keyboard::Modifiers) -> iced::event::Event {
        iced::event::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    fn shortcut(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Message> {
        keyboard_shortcuts(
            press(key, modifiers),
            iced::event::Status::Ignored,
            window::Id::unique(),
        )
    }

    #[test]
    fn control_shortcuts_switch_tabs_and_focus() {
        let ctrl = keyboard::Modifiers::CTRL;
        assert!(matches!(
            shortcut(keyboard::Key::Character("1".into()), ctrl),
            Some(Message::SelectTab(Tab::Messages))
        ));
        assert!(matches!(
            shortcut(keyboard::Key::Character("3".into()), ctrl),
            Some(Message::SelectTab(Tab::Map))
        ));
        assert!(matches!(
            shortcut(keyboard::Key::Character("f".into()), ctrl),
            Some(Message::FocusSearch)
        ));
        assert!(matches!(
            shortcut(keyboard::Key::Character("q".into()), ctrl),
            Some(Message::Quit)
        ));
    }

    #[test]
    fn escape_and_tab_are_mapped() {
        let none = keyboard::Modifiers::NONE;
        assert!(matches!(
            shortcut(keyboard::Key::Named(keyboard::key::Named::Escape), none),
            Some(Message::EscapePressed)
        ));
        assert!(matches!(
            shortcut(keyboard::Key::Named(keyboard::key::Named::Tab), none),
            Some(Message::FocusNext)
        ));
        assert!(matches!(
            shortcut(
                keyboard::Key::Named(keyboard::key::Named::Tab),
                keyboard::Modifiers::SHIFT
            ),
            Some(Message::FocusPrevious)
        ));
    }

    #[test]
    fn plain_typing_is_not_hijacked() {
        // A bare character reaches the focused text field untouched.
        assert!(
            shortcut(
                keyboard::Key::Character("a".into()),
                keyboard::Modifiers::NONE
            )
            .is_none()
        );

        // And an event a widget already consumed is never turned into an action.
        assert!(
            keyboard_shortcuts(
                press(
                    keyboard::Key::Character("q".into()),
                    keyboard::Modifiers::CTRL
                ),
                iced::event::Status::Captured,
                window::Id::unique(),
            )
            .is_none()
        );
    }
}
