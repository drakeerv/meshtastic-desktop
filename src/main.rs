//! Meshtastic Desktop - an open-source Meshtastic client for Linux.
//!
//! The binary owns the UI (iced) and wires it to the Tokio-based backend
//! (`mt-core`, `mt-transport`, `mt-persistence`) through a single event
//! bridge.

mod app;
mod assets;
mod bridge;
mod config_editor;
mod format;
mod geoclue;
mod host;
mod icons;
mod location;
mod map;
mod security;
mod settings;
mod theme;
mod tiles;
mod tray;
mod views;
mod widgets;

use app::App;
use mt_core::CoreConfig;
use mt_transport::spawn_discovery;

fn main() -> iced::Result {
    init_tracing();

    // One long-lived Tokio runtime backs the whole application. Leaking it
    // keeps the runtime alive for the process lifetime; entering it makes
    // `tokio::spawn` in the backend target this runtime regardless of the
    // thread iced calls us on.
    let runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("mt-backend")
            .build()
            .expect("failed to build the tokio runtime"),
    ));
    let _guard = runtime.enter();

    let settings = settings::AppSettings::load();

    let core_config = CoreConfig {
        data_dir: mt_persistence::Database::default_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from(".")),
        auto_reconnect: settings.auto_connect,
        ..CoreConfig::default()
    };

    let core = mt_core::spawn_core(core_config);
    let discovery = spawn_discovery();

    let core_for_boot = core.clone();
    let discovery_for_boot = discovery.clone();
    let settings_for_boot = settings.clone();

    iced::daemon(
        move || {
            let (mut app, task) = App::new(
                core_for_boot.clone(),
                discovery_for_boot.clone(),
                settings_for_boot.clone(),
            );
            app.auto_connect();
            (app, task)
        },
        App::update,
        view,
    )
    .font(iced_fonts::LUCIDE_FONT_BYTES)
    .title(|_: &App, _window: iced::window::Id| "Meshtastic".to_string())
    .theme(|app: &App, _window: iced::window::Id| app.theme())
    .subscription(|app: &App| app.subscription())
    .antialiasing(true)
    .run()
}

/// The daemon's view function.
///
/// A named function is needed here so it stays generic over the borrow
/// lifetime; an inline closure is inferred with a single concrete lifetime,
/// which the `ViewFn` bound rejects.
fn view(app: &App, _window: iced::window::Id) -> iced::Element<'_, app::Message> {
    app.view()
}

/// The main window's settings.
///
/// The app runs as a daemon (no default window) so it can keep living in the
/// tray, so the window is opened, and reopened from the tray, with these.
pub fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(1180.0, 780.0),
        icon: window_icon(),
        // Close requests are the app's to handle: with "close to tray" on we
        // close the window and stay in the tray, otherwise we exit explicitly.
        exit_on_close_request: false,
        platform_specific: platform_specific(),
        ..Default::default()
    }
}

/// The application id that compositors (KDE/GNOME) use to associate the
/// window with its `.desktop` file and icon.
pub const APP_ID: &str = "org.meshtastic.Meshtastic";

fn platform_specific() -> iced::window::settings::PlatformSpecific {
    let mut settings = iced::window::settings::PlatformSpecific::default();
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        settings.application_id = APP_ID.to_string();
    }
    settings
}

/// Decode the bundled app icon for the window/taskbar.
fn window_icon() -> Option<iced::window::Icon> {
    let decoded = image::load_from_memory(crate::assets::app_icon_png()).ok()?;
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    iced::window::icon::from_rgba(rgba.into_raw(), width, height).ok()
}

/// Set up tracing with a sensible default filter.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "meshtastic=info,mt_core=info,mt_transport=info,mt_persistence=warn,iced=warn",
        )
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
