//! Single-instance guard over the session bus.
//!
//! The first launch claims the `org.meshtastic.Meshtastic` session-bus name and
//! exports a `Show` method. A later launch fails to claim the name, asks the
//! running instance to show its window, and exits before starting any backend.
//! That keeps a tray daemon from being started twice, and makes launching the
//! app again resurface the window instead of doing nothing.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::broadcast;

use crate::app::Message;

/// The bus name, matching [`crate::APP_ID`].
const NAME: &str = "org.meshtastic.Meshtastic";
const PATH: &str = "/org/meshtastic/Meshtastic";
const IFACE: &str = "org.meshtastic.Meshtastic";

/// A handle to the running instance's guard.
///
/// Holding it keeps the bus name claimed and the `Show` object exported. When
/// the session bus is unavailable the handle is inert and the guard is skipped.
#[derive(Clone)]
pub struct InstanceHandle {
    id: u64,
    events: broadcast::Sender<()>,
    /// Kept alive so the name and exported object persist for the process.
    _connection: Option<zbus::Connection>,
}

impl InstanceHandle {
    fn new(connection: Option<zbus::Connection>, events: broadcast::Sender<()>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            events,
            _connection: connection,
        }
    }

    /// The stream of "show the window" requests, for the app's subscriptions.
    pub fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::run_with(self.clone(), stream_show)
    }
}

impl std::hash::Hash for InstanceHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// The interface a second launch calls into.
struct Show {
    events: broadcast::Sender<()>,
}

#[zbus::interface(name = "org.meshtastic.Meshtastic")]
impl Show {
    /// Ask the running instance to show its main window.
    async fn show(&self) {
        let _ = self.events.send(());
    }
}

/// Claim the single-instance name, or ask the running instance to show.
///
/// Returns `Err(())` when another instance owns the name; the caller should
/// exit. A missing session bus is not fatal: the guard is simply disabled.
pub async fn acquire() -> Result<InstanceHandle, ()> {
    let (events, _) = broadcast::channel(4);

    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(%error, "no session bus; single-instance guard disabled");
            return Ok(InstanceHandle::new(None, events));
        }
    };

    if let Err(error) = connection
        .object_server()
        .at(
            PATH,
            Show {
                events: events.clone(),
            },
        )
        .await
    {
        tracing::warn!(%error, "could not export the single-instance interface");
        return Ok(InstanceHandle::new(Some(connection), events));
    }

    match connection
        .request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner)
        | Ok(zbus::fdo::RequestNameReply::AlreadyOwner) => {
            Ok(InstanceHandle::new(Some(connection), events))
        }
        // The name is taken (zbus reports the `Exists` reply as `NameTaken`):
        // ask the owner to show its window and bow out. We never set
        // `ReplaceExisting`, so a second launch cannot steal the name.
        Err(zbus::Error::NameTaken) | Ok(zbus::fdo::RequestNameReply::Exists) => {
            if let Err(error) = ask_show(&connection).await {
                tracing::warn!(%error, "could not ask the running instance to show");
            }
            Err(())
        }
        Ok(reply) => {
            tracing::warn!(?reply, "unexpected single-instance name reply");
            Ok(InstanceHandle::new(Some(connection), events))
        }
        Err(error) => {
            tracing::warn!(%error, "could not claim the single-instance name");
            Ok(InstanceHandle::new(Some(connection), events))
        }
    }
}

/// Ask the running instance to show its window.
async fn ask_show(connection: &zbus::Connection) -> zbus::Result<()> {
    let proxy = zbus::Proxy::new(connection, NAME, PATH, IFACE).await?;
    proxy.call_method("Show", &()).await?;
    Ok(())
}

/// The installed handle, read by the app's subscription list.
static HANDLE: OnceLock<InstanceHandle> = OnceLock::new();

/// Install the running instance's handle.
pub fn install(handle: InstanceHandle) {
    let _ = HANDLE.set(handle);
}

/// The installed handle, if the guard is active.
pub fn handle() -> Option<&'static InstanceHandle> {
    HANDLE.get()
}

/// Turn "show" requests into iced messages.
fn stream_show(handle: &InstanceHandle) -> impl iced::futures::Stream<Item = Message> + use<> {
    let mut events = handle.events.subscribe();
    iced::stream::channel(4, async move |mut output| {
        use iced::futures::SinkExt;
        use tokio::sync::broadcast::error::RecvError;

        loop {
            match events.recv().await {
                Ok(()) => {
                    if output.send(Message::ShowWindow).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    })
}
