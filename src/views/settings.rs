//! Settings: a category hub with separate pages for client preferences,
//! device configuration and channels. Device-dependent pages are disabled
//! (greyed out) while no node is connected.

use iced::widget::{
    Space, button, checkbox, column, container, pick_list, row, scrollable, slider, text,
    text_input,
};
use iced::{Alignment, Element, Font, Length, Padding};

use crate::app::{App, Message, SettingsPage};
use crate::format;
use crate::icons::lucide;
use crate::settings::ThemePref;
use crate::theme;
use crate::widgets;

pub fn view(app: &App) -> Element<'_, Message> {
    // Editors take precedence over the settings pages.
    if app.editor.is_some() {
        return crate::views::config::editor_view(app);
    }
    if app.channel_editor.is_some() {
        return crate::views::config::channel_editor_view(app);
    }

    match app.settings_page {
        SettingsPage::Hub => hub(app),
        SettingsPage::App => app_page(app),
        SettingsPage::Device => device_page(app),
        SettingsPage::Channels => channels_page(app),
    }
}

// Hub

fn hub(app: &App) -> Element<'_, Message> {
    let connected = app.is_connected();
    let device_note = if connected {
        "Owner, radio and module configuration, device actions.".to_string()
    } else {
        "Connect a device to change its settings.".to_string()
    };
    let channel_note = if connected {
        format!(
            "{} slots configured on the connected node.",
            app.channels.len()
        )
    } else {
        "Connect a device to edit its channels.".to_string()
    };

    let body = column![
        category_card(
            "App settings",
            "Theme, units, notifications and connection behaviour.",
            SettingsPage::App,
            true,
        ),
        category_card(
            "Device settings",
            device_note,
            SettingsPage::Device,
            connected
        ),
        category_card(
            "Channel settings",
            channel_note,
            SettingsPage::Channels,
            connected
        ),
    ]
    .spacing(12)
    .width(Length::Fill);

    page(
        header(
            "Settings",
            "Client preferences and device configuration",
            None,
        ),
        body,
    )
}

fn category_card(
    title: &'static str,
    description: impl Into<String>,
    page: SettingsPage,
    enabled: bool,
) -> Element<'static, Message> {
    let description = description.into();
    let glyph = match page {
        SettingsPage::App => lucide::sliders_horizontal(),
        SettingsPage::Device => lucide::radio_tower(),
        SettingsPage::Channels => lucide::hash(),
        SettingsPage::Hub => lucide::settings(),
    };
    let accent = if enabled {
        theme::primary()
    } else {
        theme::text_faint()
    };
    let title_color = if enabled {
        theme::text()
    } else {
        theme::text_faint()
    };

    let inner = row![
        container(glyph.size(22).color(accent))
            .width(Length::Fixed(44.0))
            .center_x(Length::Fixed(44.0))
            .center_y(Length::Fixed(44.0))
            .style(move |_| iced::widget::container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    accent.r, accent.g, accent.b, 0.10,
                ))),
                border: iced::Border {
                    color: accent,
                    width: 1.0,
                    radius: 10.0.into(),
                },
                ..Default::default()
            }),
        column![
            text(title).size(16).color(title_color),
            text(description).size(12).color(theme::text_muted()),
        ]
        .width(Length::Fill)
        .spacing(2),
        lucide::chevron_right().size(20).color(accent),
    ]
    .spacing(14)
    .align_y(Alignment::Center);

    let mut card = button(inner)
        .width(Length::Fill)
        .padding(Padding::from([14, 16]))
        .style(if enabled {
            theme::card_button
        } else {
            theme::card_button_disabled
        });
    if enabled {
        card = card.on_press(Message::SettingsOpen(page));
    }
    card.into()
}

// App settings

fn app_page(app: &App) -> Element<'_, Message> {
    let mut body = column![].spacing(18).width(Length::Fill);

    body = body.push(section(
        "Appearance",
        column![
            setting_row(
                "Theme",
                "Choose the interface colours.",
                pick_list(
                    ThemePref::ALL.to_vec(),
                    Some(app.settings.theme),
                    Message::ThemeChanged,
                )
                .text_size(13)
                .padding(Padding::from([7, 10]))
                .width(Length::Fixed(150.0))
                .into(),
            ),
            setting_row(
                "Imperial units",
                "Show distances in miles and feet.",
                checkbox(app.settings.imperial)
                    .on_toggle(Message::ToggleImperial)
                    .into(),
            ),
            setting_row(
                "High contrast",
                "Stronger borders and secondary text.",
                checkbox(app.settings.high_contrast)
                    .on_toggle(Message::ToggleHighContrast)
                    .into(),
            ),
            setting_row(
                "Interface scale",
                "Scale the whole interface up or down. Applies when you release the slider.",
                row![
                    slider(0.75..=2.0, app.ui_scale_draft, Message::UiScalePreview)
                        .step(0.05_f32)
                        .on_release(Message::UiScaleCommitted)
                        .width(Length::Fixed(200.0)),
                    text(format!("{:.0}%", app.ui_scale_draft * 100.0))
                        .size(12)
                        .color(theme::text_muted())
                        .width(Length::Fixed(48.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            ),
        ]
        .spacing(12)
        .into(),
    ));

    body = body.push(section(
        "Map",
        setting_row(
            "Online OpenStreetMap tiles",
            "Download map imagery from OpenStreetMap while the Map view is open.",
            checkbox(app.settings.online_tiles)
                .on_toggle(Message::ToggleOnlineTiles)
                .into(),
        ),
    ));

    body = body.push(section(
        "Behaviour",
        column![
            setting_row(
                "Desktop notifications",
                "Notify when a new message arrives.",
                checkbox(app.settings.notifications)
                    .on_toggle(Message::ToggleNotifications)
                    .into(),
            ),
            setting_row(
                "Reconnect automatically",
                "Retry the last device after an unexpected disconnect.",
                checkbox(app.settings.auto_connect)
                    .on_toggle(Message::ToggleAutoConnect)
                    .into(),
            ),
            setting_row(
                "Send with Enter",
                "Send on Enter (off: send with Ctrl+Enter).",
                checkbox(app.settings.send_on_enter)
                    .on_toggle(Message::ToggleSendOnEnter)
                    .into(),
            ),
            setting_row(
                "Scan Bluetooth on opening Connect",
                "Start a BLE scan automatically when the Connect tab is opened.",
                checkbox(app.settings.scan_ble_on_start)
                    .on_toggle(Message::ToggleScanOnStart)
                    .into(),
            ),
            setting_row(
                "Close to tray",
                "Keep running in the system tray when the window is closed.",
                checkbox(app.settings.close_to_tray)
                    .on_toggle(Message::ToggleCloseToTray)
                    .into(),
            ),
        ]
        .spacing(12)
        .into(),
    ));

    let firmware = app
        .metadata
        .as_ref()
        .map(|m| m.firmware_version.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "not connected".to_string());

    body = body.push(section(
        "About",
        column![
            row![
                container(
                    iced::widget::Svg::new(crate::assets::logo(app.is_light()))
                        .width(Length::Fixed(58.0))
                        .height(Length::Fixed(32.0)),
                )
                .width(Length::Fixed(76.0))
                .height(Length::Fixed(76.0))
                .center_x(Length::Fixed(76.0))
                .center_y(Length::Fixed(76.0))
                .style(theme::inset),
                column![
                    text("Meshtastic Desktop").size(18).color(theme::text()),
                    text(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .size(12)
                        .color(theme::text_muted()),
                    text("An open-source Meshtastic client for Linux, built with Rust and iced.")
                        .size(12)
                        .color(theme::text_muted()),
                ]
                .spacing(3)
                .width(Length::Fill),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
            widgets::divider(),
            row![
                widgets::stat("Version", env!("CARGO_PKG_VERSION").to_string()),
                widgets::stat("Firmware", firmware),
                widgets::stat(
                    "Protocol",
                    format!("protobufs {}", mt_protocol::PROTOBUF_VERSION),
                ),
                widgets::stat("License", "MIT".to_string()),
            ]
            .spacing(24),
            widgets::divider(),
            column![
                link_row(
                    lucide::globe().size(14).color(theme::primary()).into(),
                    "Official Meshtastic website",
                    "https://meshtastic.org",
                ),
                link_row(
                    lucide::download().size(14).color(theme::primary()).into(),
                    "Web firmware flasher",
                    "https://flasher.meshtastic.org",
                ),
                link_row(
                    lucide::github().size(14).color(theme::primary()).into(),
                    "GitHub · drakeerv",
                    "https://github.com/drakeerv",
                ),
            ]
            .spacing(2),
            text("Not affiliated with Meshtastic LLC. \"Meshtastic\" and the Meshtastic logo are trademarks of Meshtastic LLC; board artwork is used under its asset licence.")
                .size(11)
                .color(theme::text_faint()),
        ]
        .spacing(16)
        .into(),
    ));

    page(
        header(
            "App settings",
            "Preferences for this client; no device required.",
            Some(Message::SettingsBack),
        ),
        body,
    )
}

// Device settings

fn device_page(app: &App) -> Element<'_, Message> {
    let connected = app.is_connected();
    let mut body = column![].spacing(18).width(Length::Fill);

    if !connected {
        body = body.push(banner(
            "No device connected",
            "Connect to a node from the Connect tab to view and change these settings.",
        ));
    }

    body = body.push(section("Device", device_info(app)));

    // Owner - only editable while connected.
    let owner = if connected {
        column![
            row![
                text_input("Long name", &app.owner_long)
                    .on_input(Message::OwnerLongChanged)
                    .style(theme::text_input_style)
                    .padding(Padding::from([9, 12]))
                    .size(13)
                    .width(Length::Fill),
                text_input("Short name", &app.owner_short)
                    .on_input(Message::OwnerShortChanged)
                    .style(theme::text_input_style)
                    .padding(Padding::from([9, 12]))
                    .size(13)
                    .width(Length::Fixed(160.0)),
                button(text("Save").size(13))
                    .padding(Padding::from([9, 16]))
                    .style(theme::primary_button)
                    .on_press(Message::SaveOwner),
            ]
            .spacing(8),
            text("The owner name is stored on the device and broadcast to the mesh.")
                .size(12)
                .color(theme::text_muted()),
        ]
        .spacing(8)
        .into()
    } else {
        text("Connect a device to edit its owner.")
            .size(13)
            .color(theme::text_faint())
            .into()
    };
    body = body.push(section("Owner", owner));

    // Radio + module configuration.
    body = body.push(section(
        "Radio configuration",
        crate::views::config::config_list(app, connected),
    ));

    // Device actions.
    body = body.push(section(
        "Device actions",
        column![
            row![
                action_button(
                    lucide::rotate_cw()
                        .size(14)
                        .color(if connected {
                            theme::text()
                        } else {
                            theme::text_faint()
                        })
                        .into(),
                    "Reboot",
                    connected,
                    Message::RebootPressed,
                ),
                action_button(
                    lucide::power()
                        .size(14)
                        .color(if connected {
                            theme::text()
                        } else {
                            theme::text_faint()
                        })
                        .into(),
                    "Shut down",
                    connected,
                    Message::ShutdownPressed,
                ),
                action_button(
                    lucide::rotate_ccw()
                        .size(14)
                        .color(if connected {
                            theme::danger()
                        } else {
                            theme::text_faint()
                        })
                        .into(),
                    "Factory reset (config)",
                    connected,
                    Message::FactoryResetPressed,
                ),
            ]
            .spacing(8),
            text("These commands are sent to the connected node. A factory reset erases its configuration.")
                .size(12)
                .color(theme::text_muted()),
        ]
        .spacing(10)
        .into(),
    ));

    // Host integration: push this computer's timezone, clock and location to
    // the device.
    body = body.push(section(
        "Host integration",
        column![
            setting_row(
                "Timezone",
                "Read this computer's timezone and send it to the device.",
                action_button(
                    lucide::globe()
                        .size(14)
                        .color(if connected {
                            theme::text()
                        } else {
                            theme::text_faint()
                        })
                        .into(),
                    "Fill from host",
                    connected,
                    Message::FillTimezoneFromHost,
                ),
            ),
            setting_row(
                "Device clock",
                "Send this computer's clock to the device. This also happens on every connect.",
                action_button(
                    lucide::timer()
                        .size(14)
                        .color(if connected {
                            theme::text()
                        } else {
                            theme::text_faint()
                        })
                        .into(),
                    "Set clock",
                    connected,
                    Message::SyncClockFromHost,
                ),
            ),
            setting_row(
                "Location",
                "Use this computer's location as the device's fixed position. Tries GeoClue2, then gpsd.",
                action_button(
                    lucide::locate_fixed()
                        .size(14)
                        .color(if connected {
                            theme::text()
                        } else {
                            theme::text_faint()
                        })
                        .into(),
                    "Use host location",
                    connected,
                    Message::UseHostLocation,
                ),
            ),
            setting_row(
                "Allow IP-based location",
                "If GeoClue2 and gpsd are unavailable, fall back to a city-level fix from an IP lookup service. This shares the public IP with a third party.",
                checkbox(app.settings.use_ip_location)
                    .on_toggle(Message::ToggleIpLocation)
                    .into(),
            ),
            setting_row(
                "Manual position",
                "Set the fixed position by hand, in decimal degrees.",
                row![
                    text_input("Latitude", &app.manual_lat)
                        .on_input(Message::ManualLatChanged)
                        .style(theme::text_input_style)
                        .padding(Padding::from([9, 12]))
                        .size(13)
                        .width(Length::Fixed(104.0)),
                    text_input("Longitude", &app.manual_lon)
                        .on_input(Message::ManualLonChanged)
                        .style(theme::text_input_style)
                        .padding(Padding::from([9, 12]))
                        .size(13)
                        .width(Length::Fixed(104.0)),
                    action_button(
                        lucide::map_pin()
                            .size(14)
                            .color(if connected {
                                theme::text()
                            } else {
                                theme::text_faint()
                            })
                            .into(),
                        "Set",
                        connected,
                        Message::SetManualPosition,
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            ),
        ]
        .spacing(12)
        .into(),
    ));

    // Diagnostics.
    let copy_logs = {
        let mut button = button(widgets::labelled(
            lucide::clipboard_copy().size(14).color(theme::text()),
            "Copy",
        ))
        .padding(Padding::from([9, 14]))
        .style(theme::secondary_button);
        if !app.logs.is_empty() {
            let all = app.logs.iter().cloned().collect::<Vec<_>>().join("\n");
            button = button.on_press(Message::CopyText(all));
        }
        button
    };
    body = body.push(section(
        "Diagnostics",
        column![
            row![
                button(widgets::labelled(
                    if app.show_logs {
                        lucide::eye_off().size(14).color(theme::text())
                    } else {
                        lucide::scroll_text().size(14).color(theme::text())
                    },
                    if app.show_logs {
                        "Hide logs"
                    } else {
                        "Show logs"
                    },
                ))
                .padding(Padding::from([9, 14]))
                .style(theme::secondary_button)
                .on_press(Message::ToggleLogs),
                copy_logs,
                button(widgets::labelled(
                    lucide::eraser().size(14).color(theme::text()),
                    "Clear",
                ))
                .padding(Padding::from([9, 14]))
                .style(theme::secondary_button)
                .on_press(Message::ClearLogs),
                text(format!("{} lines", app.logs.len()))
                    .size(12)
                    .color(theme::text_muted()),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            if app.show_logs {
                log_view(app)
            } else {
                Space::new().height(0).into()
            },
        ]
        .spacing(10)
        .into(),
    ));

    page(
        header(
            "Device settings",
            "Owner, radio configuration and device actions.",
            Some(Message::SettingsBack),
        ),
        body,
    )
}

// Channel settings

fn channels_page(app: &App) -> Element<'_, Message> {
    let connected = app.is_connected();
    let mut body = column![].spacing(18).width(Length::Fill);

    if !connected {
        body = body.push(banner(
            "No device connected",
            "Connect to a node from the Connect tab to edit its channels.",
        ));
    }

    body = body.push(section(
        "Channels",
        crate::views::config::channel_list(app, connected),
    ));

    page(
        header(
            "Channel settings",
            "The eight channel slots on the connected node.",
            Some(Message::SettingsBack),
        ),
        body,
    )
}

// Shared pieces

/// The frame shared by every settings page: a fixed-height header followed by
/// scrollable content. Using one frame for the hub and its sub-pages keeps the
/// title and content from shifting when navigating between them.
pub(crate) fn page<'a>(
    header: Element<'a, Message>,
    body: iced::widget::Column<'a, Message>,
) -> Element<'a, Message> {
    container(
        scrollable(
            column![
                header,
                container(body.padding(Padding::from([4, 24]))).width(Length::Fill),
            ]
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::content)
    .into()
}

/// The settings header. A back button is shown when `back` is set; otherwise
/// the same space is reserved invisibly, and the height is fixed, so the hub,
/// sub-pages and editors line up exactly.
pub(crate) fn header(
    title: impl Into<String>,
    subtitle: impl Into<String>,
    back: Option<Message>,
) -> Element<'static, Message> {
    let leading: Element<'static, Message> = match back {
        Some(message) => button(
            row![
                lucide::chevron_left().size(15).color(theme::text_muted()),
                text("Back").size(13),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
        .width(Length::Fixed(86.0))
        .padding(Padding::from([7, 12]))
        .style(theme::ghost_button)
        .on_press(message)
        .into(),
        None => Space::new().width(Length::Fixed(86.0)).into(),
    };

    container(
        row![
            leading,
            column![
                text(title.into()).size(20).color(theme::text()),
                text(subtitle.into()).size(12).color(theme::text_muted()),
            ]
            .width(Length::Fill)
            .spacing(2),
        ]
        .spacing(14)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(60.0))
    .padding(Padding::from([0, 22]))
    .align_y(Alignment::Center)
    .into()
}

fn banner(title: &'static str, message: &'static str) -> Element<'static, Message> {
    container(
        column![
            text(title).size(14).color(theme::warning()),
            text(message).size(12).color(theme::text_muted()),
        ]
        .spacing(2),
    )
    .width(Length::Fill)
    .padding(Padding::from([12, 16]))
    .style(|_: &iced::Theme| iced::widget::container::Style {
        background: Some(theme::surface_alt().into()),
        border: iced::Border {
            color: theme::warning(),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    })
    .into()
}

fn section<'a>(title: &'a str, content: Element<'a, Message>) -> Element<'a, Message> {
    container(
        column![
            text(title.to_uppercase())
                .size(11)
                .color(theme::text_faint()),
            content,
        ]
        .spacing(12),
    )
    .width(Length::Fill)
    .padding(Padding::from([18, 20]))
    .style(theme::card)
    .into()
}

/// A full-width clickable external link row, opened in the system browser.
fn link_row(
    icon: Element<'static, Message>,
    label: &'static str,
    url: &'static str,
) -> Element<'static, Message> {
    button(
        row![
            icon,
            text(label).size(13).color(theme::text()),
            Space::new().width(Length::Fill),
            lucide::arrow_up_right().size(12).color(theme::text_faint()),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([8, 10]))
    .style(theme::ghost_button)
    .on_press(Message::OpenLink(url.to_string()))
    .into()
}

fn setting_row<'a>(
    title: &'a str,
    description: &'a str,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    row![
        column![
            text(title).size(14).color(theme::text()),
            text(description).size(12).color(theme::text_muted()),
        ]
        .width(Length::Fill)
        .spacing(2),
        control,
    ]
    .align_y(Alignment::Center)
    .spacing(12)
    .into()
}

/// A button that greys out and becomes inert when `enabled` is false.
fn action_button(
    icon: Element<'static, Message>,
    label: &'static str,
    enabled: bool,
    message: Message,
) -> Element<'static, Message> {
    let style: fn(&iced::Theme, iced::widget::button::Status) -> iced::widget::button::Style =
        if enabled {
            theme::secondary_button
        } else {
            theme::ghost_button
        };
    let mut view = button(widgets::labelled(icon, label))
        .padding(Padding::from([9, 14]))
        .style(style);
    if enabled {
        view = view.on_press(message);
    }
    view.into()
}

fn device_info(app: &App) -> Element<'_, Message> {
    let firmware = app
        .metadata
        .as_ref()
        .map(|m| m.firmware_version.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "-".to_string());

    let (hw, region) = match app.my_node_num.and_then(|n| app.node(n)) {
        Some(node) => {
            let hw = node
                .user
                .as_ref()
                .map(|u| format::hw_model_label(u.hw_model).to_string())
                .unwrap_or_else(|| "-".to_string());
            (hw, region_label(app))
        }
        None => ("-".to_string(), region_label(app)),
    };

    let node = app
        .my_node_num
        .map(|n| format!("{} · {}", app.node_name(n), format::node_id(n)))
        .unwrap_or_else(|| "Not connected".to_string());

    column![
        row![
            widgets::stat("Node", node),
            widgets::stat("Firmware", firmware),
            widgets::stat("Hardware", hw),
            widgets::stat("Region", region),
        ]
        .spacing(16),
        text(format!(
            "Channels: {} · Nodes known: {} · Config sections: {} + {}",
            app.channels.len(),
            app.nodes.len(),
            app.device_configs.len(),
            app.module_configs.len(),
        ))
        .size(12)
        .color(theme::text_muted()),
    ]
    .spacing(12)
    .into()
}

fn region_label(app: &App) -> String {
    use meshtastic_protobufs::meshtastic::config;
    app.device_configs
        .iter()
        .find_map(|c| match &c.payload_variant {
            Some(config::PayloadVariant::Lora(lora)) => {
                config::lo_ra_config::RegionCode::try_from(lora.region)
                    .ok()
                    .map(|region| format!("{region:?}"))
            }
            _ => None,
        })
        .unwrap_or_else(|| "-".to_string())
}

fn log_view(app: &App) -> Element<'_, Message> {
    // Only the most recent lines are interactive; Copy takes the whole buffer.
    const VISIBLE: usize = 200;
    let start = app.logs.len().saturating_sub(VISIBLE);

    let mut lines = column![].spacing(1).width(Length::Fill);
    if start > 0 {
        lines = lines.push(
            text(format!(
                "... {start} earlier lines hidden (Copy copies all)"
            ))
            .size(10)
            .color(theme::text_faint()),
        );
    }
    for line in app.logs.iter().skip(start) {
        lines = lines.push(
            button(
                text(line)
                    .size(11)
                    .font(Font::MONOSPACE)
                    .color(log_level_color(line))
                    .width(Length::Fill),
            )
            .width(Length::Fill)
            .padding(Padding::from([1, 4]))
            .style(theme::ghost_button)
            .on_press(Message::CopyText(line.clone())),
        );
    }

    container(scrollable(lines).height(220).width(Length::Fill))
        .width(Length::Fill)
        .padding(Padding::from(10))
        .style(theme::inset)
        .into()
}

/// Colour a device log line by its level. The firmware emits ANSI colours that
/// we strip for display, so the level is recovered from the text instead.
fn log_level_color(line: &str) -> iced::Color {
    if line.contains("ERROR") || line.contains("[E]") {
        theme::danger()
    } else if line.contains("WARN") || line.contains("[W]") {
        theme::warning()
    } else if line.contains("DEBUG") || line.contains("[D]") {
        theme::text_faint()
    } else if line.contains("INFO") || line.contains("[I]") {
        theme::text_muted()
    } else {
        theme::text_muted()
    }
}
