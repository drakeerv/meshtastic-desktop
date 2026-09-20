//! System tray integration.
//!
//! A StatusNotifierItem (through `ksni`) with the app icon, a status tooltip
//! and a small menu. The tray runs on its own thread with its own Tokio
//! runtime and talks to the UI in both directions:
//!
//! - tray to app: a broadcast channel carrying [`TrayEvent`]s, folded into the
//!   app's subscriptions;
//! - app to tray: a shared state cell plus a refresh channel that makes the
//!   tray re-read the tooltip and emit a change.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip, Tray, TrayMethods};
use tokio::sync::{broadcast, mpsc};

use crate::app::Message;

/// Events the tray raises for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    /// Show or hide the main window.
    Toggle,
    /// Quit the application.
    Quit,
}

/// What the tray shows in its tooltip and status menu entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayState {
    /// "Connected" or "Disconnected".
    pub status: String,
    /// A short stats line, e.g. "12 nodes · 8 channels · fw 2.7.26".
    pub stats: String,
}

impl TrayState {
    /// The one-line summary used by the tooltip and the menu header.
    fn summary(&self) -> String {
        match (self.status.is_empty(), self.stats.is_empty()) {
            (true, _) => "Starting".to_string(),
            (false, true) => self.status.clone(),
            (false, false) => format!("{} · {}", self.status, self.stats),
        }
    }
}

/// The app side of the tray: drives it and subscribes to its events.
#[derive(Clone)]
pub struct TrayHandle {
    id: u64,
    state: Arc<Mutex<TrayState>>,
    refresh: mpsc::UnboundedSender<()>,
    events: broadcast::Sender<TrayEvent>,
}

impl TrayHandle {
    /// Start the tray service on a dedicated thread.
    pub fn spawn() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);

        let state = Arc::new(Mutex::new(TrayState::default()));
        let (refresh, refresh_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(16);

        let tray = MeshtasticTray {
            state: Arc::clone(&state),
            events: events.clone(),
        };
        if let Err(error) = std::thread::Builder::new()
            .name("mt-tray".to_string())
            .spawn(move || run(tray, refresh_rx))
        {
            tracing::warn!(%error, "could not start the tray thread");
        }

        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            state,
            refresh,
            events,
        }
    }

    /// Replace the tooltip state and ask the tray to refresh.
    pub fn update(&self, state: TrayState) {
        if let Ok(mut current) = self.state.lock() {
            *current = state;
        }
        let _ = self.refresh.send(());
    }

    /// The stream of tray events, for the app's subscription list.
    pub fn subscription(&self) -> iced::Subscription<Message> {
        iced::Subscription::run_with(self.clone(), stream_tray)
    }
}

impl std::hash::Hash for TrayHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Run the tray service until the refresh channel closes (app exit).
fn run(tray: MeshtasticTray, mut refresh: mpsc::UnboundedReceiver<()>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::warn!(%error, "could not start the tray runtime");
            return;
        }
    };

    runtime.block_on(async move {
        let handle = match tray.spawn().await {
            Ok(handle) => handle,
            Err(error) => {
                tracing::warn!(%error, "system tray unavailable");
                return;
            }
        };
        while refresh.recv().await.is_some() {
            handle.update(|_| ()).await;
        }
    });
}

/// Multiplex the tray's broadcast channel into iced messages.
fn stream_tray(tray: &TrayHandle) -> impl iced::futures::Stream<Item = Message> + use<> {
    let mut events = tray.events.subscribe();
    iced::stream::channel(16, async move |mut output| {
        use iced::futures::SinkExt;
        use tokio::sync::broadcast::error::RecvError;

        loop {
            match events.recv().await {
                Ok(event) => {
                    if output.send(Message::Tray(event)).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    })
}

/// The StatusNotifierItem itself.
#[derive(Clone)]
struct MeshtasticTray {
    state: Arc<Mutex<TrayState>>,
    events: broadcast::Sender<TrayEvent>,
}

impl Tray for MeshtasticTray {
    fn id(&self) -> String {
        crate::APP_ID.to_string()
    }

    fn title(&self) -> String {
        "Meshtastic Desktop".to_string()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![app_icon()]
    }

    fn tool_tip(&self) -> ToolTip {
        let description = self
            .state
            .lock()
            .map(|state| state.summary())
            .unwrap_or_else(|_| "Starting".to_string());
        ToolTip {
            title: "Meshtastic Desktop".to_string(),
            description,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let summary = self
            .state
            .lock()
            .map(|state| state.summary())
            .unwrap_or_else(|_| "Starting".to_string());

        vec![
            // A non-clickable header showing the current status and stats.
            MenuItem::Standard(StandardItem {
                label: summary,
                enabled: false,
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Show / hide window".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.events.send(TrayEvent::Toggle);
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Quit".to_string(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.events.send(TrayEvent::Quit);
                }),
                ..Default::default()
            }),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.send(TrayEvent::Toggle);
    }
}

/// The bundled app icon, downscaled and converted to ARGB32 for the tray.
fn app_icon() -> Icon {
    use std::sync::OnceLock;
    static ICON: OnceLock<Icon> = OnceLock::new();

    ICON.get_or_init(|| {
        let Some(decoded) = image::load_from_memory(crate::assets::app_icon_png()).ok() else {
            return Icon {
                width: 0,
                height: 0,
                data: Vec::new(),
            };
        };
        let rgba = decoded
            .resize(64, 64, image::imageops::FilterType::Lanczos3)
            .into_rgba8();
        let (width, height) = rgba.dimensions();
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for pixel in rgba.pixels() {
            let [r, g, b, a] = pixel.0;
            // StatusNotifierItem pixmaps are ARGB32 in network byte order.
            data.extend_from_slice(&[a, r, g, b]);
        }
        Icon {
            width: width as i32,
            height: height as i32,
            data,
        }
    })
    .clone()
}
