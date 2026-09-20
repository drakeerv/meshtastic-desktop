//! Connect view: discovered devices, manual addresses and link status.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, ConnectTab, Message};
use crate::icons::lucide;
use crate::theme;
use crate::widgets;
use mt_core::{ConnectionState, TransportKind};

pub fn view(app: &App) -> Element<'_, Message> {
    let header_actions = vec![
        if app.ble_scanning {
            button(widgets::labelled(
                lucide::square().size(14).color(theme::text()),
                "Stop scan",
            ))
            .padding(Padding::from([7, 14]))
            .style(theme::secondary_button)
            .on_press(Message::StopScan)
            .into()
        } else {
            button(widgets::labelled(
                lucide::bluetooth_searching()
                    .size(14)
                    .color(iced::Color::from_rgb8(9, 20, 14)),
                "Scan Bluetooth",
            ))
            .padding(Padding::from([7, 14]))
            .style(theme::primary_button)
            .on_press(Message::StartScan)
            .into()
        },
        if app.is_connected() {
            button(widgets::labelled(
                lucide::refresh_cw().size(14).color(theme::text()),
                "Resync",
            ))
            .padding(Padding::from([7, 14]))
            .style(theme::secondary_button)
            .on_press(Message::ResyncPressed)
            .into()
        } else {
            Space::new().width(0).into()
        },
        if app.is_connected() {
            button(widgets::labelled(
                lucide::unplug().size(14).color(theme::danger()),
                "Disconnect",
            ))
            .padding(Padding::from([7, 14]))
            .style(theme::danger_button)
            .on_press(Message::DisconnectPressed)
            .into()
        } else {
            Space::new().width(0).into()
        },
    ];

    let mut body = column![
        widgets::page_header(
            "Connect",
            Some("Bluetooth, USB serial and WiFi nodes".to_string()),
            header_actions,
        ),
        container(connection_card(app))
            .width(Length::Fill)
            .padding(Padding::from([0, 22])),
        container(manual_entry(app))
            .width(Length::Fill)
            .padding(Padding::from([12, 22])),
        container(tab_bar(app))
            .width(Length::Fill)
            .padding(Padding::from([0, 22])),
    ]
    .spacing(8)
    .width(Length::Fill)
    .height(Length::Fill);

    body = body.push(
        container(device_list(app))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::from([8, 22])),
    );

    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::content)
        .into()
}

fn connection_card(app: &App) -> Element<'_, Message> {
    let (color, label) = state_color(&app.conn);
    let connected = app.is_connected();

    // Board artwork: the connected node's model, or a generic placeholder.
    let model = app
        .my_node_num
        .and_then(|n| app.node(n))
        .and_then(|node| node.user.as_ref())
        .map(|user| user.hw_model);
    let art = match model {
        Some(model) => crate::assets::device_art(model),
        None => crate::assets::unknown_art(),
    };

    let title = app
        .my_node_num
        .map(|n| format!("{} ({})", app.node_name(n), crate::format::node_id(n)))
        .unwrap_or_else(|| "No device connected".to_string());

    let subtitle = app
        .conn
        .address()
        .map(|a| a.to_string())
        .unwrap_or_else(|| {
            "Pick a discovered device below, or enter an address by hand.".to_string()
        });

    let mut details = row![widgets::tag(label, color)]
        .spacing(10)
        .align_y(Alignment::Center);
    if let Some(address) = app.conn.address() {
        details = details.push(widgets::tag(address.kind().as_str(), theme::text_muted()));
    }

    let mut stats = row![].spacing(18);
    if connected {
        stats = stats.push(widgets::stat(
            "Firmware",
            app.metadata
                .as_ref()
                .map(|m| m.firmware_version.clone())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "-".to_string()),
        ));
        stats = stats.push(widgets::stat("Channels", format!("{}", app.channels.len())));
        stats = stats.push(widgets::stat("Nodes", format!("{}", app.nodes.len())));
    } else {
        stats = stats.push(widgets::stat(
            "Tip",
            "Use the address field or scan for Bluetooth nodes.".to_string(),
        ));
    }

    container(
        row![
            container(
                iced::widget::Svg::new(art)
                    .width(Length::Fixed(128.0))
                    .height(Length::Fixed(128.0))
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Length::Fixed(128.0))
            .height(Length::Fixed(128.0))
            .center_x(Length::Fixed(128.0))
            .center_y(Length::Fixed(128.0)),
            column![
                details,
                text(title).size(18).color(theme::text()),
                text(subtitle).size(12).color(theme::text_muted()),
                Space::new().height(Length::Fixed(4.0)),
                stats,
            ]
            .spacing(6)
            .width(Length::Fill),
        ]
        .spacing(18)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(160.0))
    .padding(Padding::from([10, 18]))
    .style(theme::card)
    .into()
}

fn state_color(state: &ConnectionState) -> (iced::Color, &'static str) {
    match state {
        ConnectionState::Disconnected => (theme::text_faint(), "offline"),
        ConnectionState::Connecting(_) => (theme::warning(), "connecting"),
        ConnectionState::Handshaking(_) => (theme::warning(), "syncing"),
        ConnectionState::Connected { .. } => (theme::primary(), "connected"),
        ConnectionState::Reconnecting { .. } => (theme::warning(), "retrying"),
        ConnectionState::Disconnecting(_) => (theme::text_muted(), "closing"),
    }
}

fn manual_entry(app: &App) -> Element<'_, Message> {
    row![
        text_input(
            "192.168.1.42  ·  /dev/ttyUSB0  ·  aa:bb:cc:dd:ee:ff",
            &app.manual_address
        )
        .on_input(Message::ManualAddressChanged)
        .on_submit(Message::ConnectManual)
        .style(theme::text_input_style)
        .padding(Padding::from([10, 12]))
        .size(14)
        .width(Length::Fill),
        button(widgets::labelled(
            lucide::plug().size(14).color(theme::text()),
            "Connect",
        ))
        .padding(Padding::from([10, 18]))
        .style(theme::secondary_button)
        .on_press(Message::ConnectManual),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// The All / Serial / Bluetooth / IP filter tabs, with device counts.
fn tab_bar(app: &App) -> Element<'_, Message> {
    let mut tabs = row![].spacing(6).align_y(Alignment::Center);
    for tab in ConnectTab::ALL {
        let active = app.connect_tab == tab;
        let count = app.device_count(tab);
        let label = if count > 0 {
            format!("{} ({count})", tab.label())
        } else {
            tab.label().to_string()
        };
        tabs = tabs.push(
            button(text(label).size(13))
                .padding(Padding::from([7, 14]))
                .style(if active {
                    theme::primary_button
                } else {
                    theme::secondary_button
                })
                .on_press(Message::ConnectTabSelected(tab)),
        );
    }
    tabs.into()
}

fn device_list(app: &App) -> Element<'_, Message> {
    let devices = app.tab_devices();
    if devices.is_empty() {
        let hint = match app.connect_tab {
            ConnectTab::Serial => "No USB serial adapters found. Plug in a node over USB.",
            ConnectTab::Bluetooth => {
                if app.ble_scanning {
                    "Scanning… make sure your node is awake and in range."
                } else {
                    "No Bluetooth nodes found yet. Start a scan."
                }
            }
            ConnectTab::Ip => "No WiFi nodes found. They appear via mDNS on the same network.",
            ConnectTab::All => {
                if app.ble_scanning {
                    "Scanning… make sure your node is awake and in range."
                } else {
                    "No devices found yet. Start a Bluetooth scan, or connect by address above."
                }
            }
        };
        return widgets::empty_state(
            lucide::bluetooth_searching()
                .size(42)
                .color(theme::text_faint())
                .into(),
            "No devices",
            hint,
        );
    }

    let mut list = column![].spacing(14).width(Length::Fill);
    for (kind, heading) in [
        (TransportKind::Ble, "Bluetooth"),
        (TransportKind::Serial, "USB serial"),
        (TransportKind::Tcp, "WiFi / TCP"),
        (TransportKind::Mock, "Demo"),
    ] {
        let mut rows = column![].spacing(6).width(Length::Fill);
        let mut found = false;
        for device in devices.iter().filter(|d| d.address.kind() == kind) {
            found = true;
            rows = rows.push(device_row(app, device));
        }
        if found {
            list = list.push(
                column![
                    text(heading.to_uppercase())
                        .size(11)
                        .color(theme::text_faint()),
                    rows,
                ]
                .spacing(6)
                .width(Length::Fill),
            );
        }
    }

    scrollable(list)
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}

fn device_row<'a>(app: &'a App, device: &'a mt_core::DiscoveredDevice) -> Element<'a, Message> {
    let address = device.address.clone();
    let connected = app
        .conn
        .address()
        .map(|a| a == &device.address)
        .unwrap_or(false);

    let mut detail = device.detail.clone();
    if let Some(rssi) = device.rssi {
        detail = format!("{detail}  ·  {rssi} dBm");
    }

    container(
        row![
            column![
                text(&device.name).size(15).color(theme::text()),
                text(detail).size(12).color(theme::text_muted()),
            ]
            .spacing(2)
            .width(Length::Fill),
            if connected {
                button(widgets::labelled(
                    lucide::unplug().size(13).color(theme::text_muted()),
                    "Connected",
                ))
                .padding(Padding::from([7, 14]))
                .style(theme::ghost_button)
                .on_press(Message::DisconnectPressed)
            } else {
                button(widgets::labelled(
                    lucide::plug()
                        .size(13)
                        .color(iced::Color::from_rgb8(9, 20, 14)),
                    "Connect",
                ))
                .padding(Padding::from([7, 14]))
                .style(theme::primary_button)
                .on_press(Message::ConnectTo(address.clone()))
            },
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([12, 14]))
    .style(theme::card)
    .into()
}
