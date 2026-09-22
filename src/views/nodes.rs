//! Nodes view: the mesh node database with per-node detail and actions.

use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::Message;
use crate::format;
use crate::icons::lucide;
use crate::settings::NodeSort;
use crate::theme;
use crate::widgets;
use meshtastic_protobufs::meshtastic::{NodeInfo, Position, User, telemetry};

/// Widget id for the node search box, so a shortcut can focus it.
pub const SEARCH_INPUT_ID: &str = "node-search";

pub fn view(app: &crate::app::App) -> Element<'_, Message> {
    row![node_list(app), node_detail(app)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn node_list(app: &crate::app::App) -> Element<'_, Message> {
    let header = container(
        column![
            row![
                text("Nodes").size(18).color(theme::text()),
                Space::new().width(Length::Fill),
                button(widgets::labelled(
                    lucide::download().size(13).color(theme::text()),
                    "Import",
                ))
                .padding(Padding::from([6, 10]))
                .style(theme::secondary_button)
                .on_press(Message::OpenContactImport),
                widgets::tag(format!("{}", app.nodes.len()), theme::primary()),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text_input("Search by name or id…", &app.node_search)
                .id(SEARCH_INPUT_ID)
                .on_input(Message::NodeSearchChanged)
                .style(theme::text_input_style)
                .padding(Padding::from([9, 12]))
                .size(13),
            row![
                text("Sort").size(12).color(theme::text_faint()),
                Space::new().width(Length::Fill),
                pick_list(
                    NodeSort::ALL.to_vec(),
                    Some(app.settings.node_sort),
                    Message::NodeSortChanged,
                )
                .text_size(12)
                .padding(Padding::from([5, 9]))
                .width(Length::Fixed(130.0)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(10),
    )
    .width(Length::Fill)
    .padding(Padding::from([16, 14]));

    let nodes = app.visible_nodes();
    let list: Element<'_, Message> = if nodes.is_empty() {
        widgets::empty_state(
            lucide::radio_tower()
                .size(42)
                .color(theme::text_faint())
                .into(),
            "No nodes",
            "Nodes appear as the device downloads its database and hears the mesh.",
        )
    } else {
        let mut items = column![].spacing(6).width(Length::Fill);
        for node in nodes {
            items = items.push(node_card(app, node));
        }
        scrollable(container(items).padding(Padding::from([4, 12])))
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    };

    container(column![header, widgets::divider(), list].height(Length::Fill))
        .width(330)
        .height(Length::Fill)
        .style(theme::panel)
        .into()
}

fn node_card<'a>(app: &'a crate::app::App, node: &'a NodeInfo) -> Element<'a, Message> {
    let selected = app.selected_node == Some(node.num);
    let name = format::node_name(node);
    let id = format::node_id(node.num);

    let mut badges = row![].spacing(6).align_y(Alignment::Center);
    if let Some(level) = node.device_metrics.as_ref().and_then(|m| m.battery_level) {
        badges = badges.push(widgets::tag(
            format::battery_label(Some(level)),
            battery_color(level),
        ));
    }
    if node.via_mqtt {
        badges = badges.push(widgets::tag("MQTT", theme::text_muted()));
    }
    if node.is_favorite {
        badges = badges.push(widgets::icon_tag(
            lucide::star().size(11).color(theme::warning()).into(),
            theme::warning(),
        ));
    }
    if node.is_ignored {
        badges = badges.push(widgets::tag("muted", theme::text_faint()));
    }
    // A lock marks a node whose public key is on file, so PKC direct
    // messages to it are end to end encrypted.
    if node
        .user
        .as_ref()
        .map(|user| has_public_key(&user.public_key))
        .unwrap_or(false)
    {
        badges = badges.push(widgets::icon_tag(
            lucide::lock().size(11).color(theme::primary()).into(),
            theme::primary(),
        ));
    }

    let model = node.user.as_ref().map(|u| u.hw_model).unwrap_or(0);

    button(
        row![
            container(
                iced::widget::Svg::new(crate::assets::device_art(model))
                    .width(Length::Fixed(40.0))
                    .height(Length::Fixed(40.0))
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Length::Fixed(40.0))
            .height(Length::Fixed(40.0))
            .center_x(Length::Fixed(40.0))
            .center_y(Length::Fixed(40.0)),
            column![
                row![
                    lucide::signal().size(12).color(signal_color(node.snr)),
                    text(name).size(14).color(theme::text()).width(Length::Fill),
                    text(format::last_heard_short(node.last_heard, app.now))
                        .size(11)
                        .color(theme::text_faint()),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                row![
                    text(id).size(11).color(theme::text_muted()),
                    Space::new().width(Length::Fill),
                    badges,
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(5)
            .width(Length::Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(Padding::from([10, 12]))
    .style(theme::nav_button(selected))
    .on_press(Message::NodeSelected(node.num))
    .into()
}

fn node_detail(app: &crate::app::App) -> Element<'_, Message> {
    let Some(num) = app.selected_node else {
        return container(widgets::empty_state(
            lucide::radio_tower()
                .size(42)
                .color(theme::text_faint())
                .into(),
            "Select a node",
            "Choose a node to see its details, telemetry and available actions.",
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::content)
        .into();
    };

    let Some(node) = app.node(num) else {
        return container(widgets::empty_state(
            lucide::radio_tower()
                .size(42)
                .color(theme::text_faint())
                .into(),
            "Node unavailable",
            "This node is no longer in the database.",
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::content)
        .into();
    };

    let mut body = column![].spacing(14);

    // identity
    let name = format::node_name(node);
    let model = node.user.as_ref().map(|u| u.hw_model).unwrap_or(0);
    body = body.push(
        row![
            container(
                iced::widget::Svg::new(crate::assets::device_art(model))
                    .width(Length::Fixed(84.0))
                    .height(Length::Fixed(84.0))
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Length::Fixed(84.0))
            .height(Length::Fixed(84.0))
            .center_x(Length::Fixed(84.0))
            .center_y(Length::Fixed(84.0)),
            column![
                text(name).size(22).color(theme::text()),
                text(format::node_id(node.num))
                    .size(13)
                    .color(theme::text_muted()),
            ]
            .width(Length::Fill),
            button(widgets::labelled(
                lucide::star().size(14).color(if node.is_favorite {
                    theme::warning()
                } else {
                    theme::text_muted()
                }),
                "Favourite",
            ))
            .padding(Padding::from([7, 12]))
            .style(theme::secondary_button)
            .on_press(Message::ToggleFavorite(node.num)),
            button(widgets::labelled(
                lucide::message_circle()
                    .size(14)
                    .color(iced::Color::from_rgb8(9, 20, 14)),
                "Direct message",
            ))
            .padding(Padding::from([7, 12]))
            .style(theme::primary_button)
            .on_press(Message::OpenConversation(node.num)),
            button(lucide::x().size(15).color(theme::text_muted()))
                .padding(Padding::from([7, 10]))
                .style(theme::ghost_button)
                .on_press(Message::NodeDeselected),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    body = body.push(widgets::divider());

    // details
    let user = node.user.as_ref();
    // Device metrics can arrive either through the node database or as a
    // telemetry packet. Prefer the freshest telemetry sample; the dedicated
    // row below is the single place they are shown.
    let metrics = app
        .latest_telemetry
        .get(&num)
        .and_then(|sample| match &sample.variant {
            Some(telemetry::Variant::DeviceMetrics(m)) => Some(m),
            _ => None,
        })
        .or(node.device_metrics.as_ref());
    let rssi = app.node_rssi.get(&num).copied();
    let public_key = user.map(|u| u.public_key.as_slice()).unwrap_or(&[]);
    let shareable = has_public_key(public_key);

    body = body.push(
        column![
            row![
                widgets::stat("Short name", non_empty(user.map(|u| u.short_name.as_str())),),
                widgets::stat(
                    "Role",
                    format::role_label(user.map(|u| u.role).unwrap_or(0)).to_string(),
                ),
                widgets::stat("Node ID", format::node_id(node.num)),
                widgets::stat("Node number", node.num.to_string()),
            ]
            .spacing(16),
            row![
                widgets::stat("User ID", non_empty(user.map(|u| u.id.as_str()))),
                widgets::stat(
                    "Hardware",
                    user.map(|u| format::hw_model_label(u.hw_model))
                        .unwrap_or("-")
                        .to_string(),
                ),
                widgets::stat(
                    "Transport",
                    format::transport_label(node.via_mqtt).to_string()
                ),
                widgets::stat("Hops", format::hops_label(node.hops_away)),
            ]
            .spacing(16),
            row![
                widgets::stat(
                    "SNR",
                    format!("{:.1} dB · {}", node.snr, format::snr_label(node.snr)),
                ),
                widgets::stat(
                    "RSSI",
                    rssi.map(|value| format!("{value} dBm"))
                        .unwrap_or_else(|| "-".to_string()),
                ),
                widgets::stat(
                    "Last heard",
                    format::relative_time(node.last_heard, app.now),
                ),
                widgets::stat(
                    "Uptime",
                    metrics
                        .and_then(|m| m.uptime_seconds)
                        .map(format_uptime)
                        .unwrap_or_else(|| "-".to_string()),
                ),
            ]
            .spacing(16),
        ]
        .spacing(14),
    );

    body = body.push(widgets::divider());
    body = body.push(security_block(
        user,
        node.is_key_manually_verified,
        Some(node.num) == app.my_node_num,
    ));
    if !public_key.is_empty() {
        body = body.push(public_key_block(public_key));
    }

    // device metrics
    body = body.push(widgets::divider());
    body = body.push(
        row![
            widgets::stat(
                "Battery",
                format::battery_label(metrics.and_then(|m| m.battery_level)),
            ),
            widgets::stat(
                "Voltage",
                metrics
                    .and_then(|m| m.voltage)
                    .map(|v| format!("{v:.2} V"))
                    .unwrap_or_else(|| "-".to_string()),
            ),
            widgets::stat(
                "Channel util",
                metrics
                    .and_then(|m| m.channel_utilization)
                    .map(|v| format!("{v:.1}%"))
                    .unwrap_or_else(|| "-".to_string()),
            ),
            widgets::stat(
                "Air util",
                metrics
                    .and_then(|m| m.air_util_tx)
                    .map(|v| format!("{v:.1}%"))
                    .unwrap_or_else(|| "-".to_string()),
            ),
        ]
        .spacing(16),
    );

    // position
    body = body.push(widgets::divider());
    body = body.push(position_block(app, node.position.as_ref()));

    // telemetry
    if let Some(sample) = app.latest_telemetry.get(&num) {
        if let Some(block) = telemetry_block(sample) {
            body = body.push(block);
        }
    }

    // traceroute
    body = body.push(widgets::divider());
    body = body.push(traceroute_block(app, num));

    // actions
    body = body.push(
        row![
            button(widgets::labelled(
                lucide::locate_fixed().size(14).color(theme::text()),
                "Request position",
            ))
            .padding(Padding::from([8, 12]))
            .style(theme::secondary_button)
            .on_press(Message::RequestPosition(num)),
            button(widgets::labelled(
                lucide::route().size(14).color(theme::text()),
                "Traceroute",
            ))
            .padding(Padding::from([8, 12]))
            .style(theme::secondary_button)
            .on_press(Message::Traceroute(num)),
            {
                let base = button(widgets::labelled(
                    lucide::qr_code().size(14).color(if shareable {
                        theme::text()
                    } else {
                        theme::text_faint()
                    }),
                    "Share contact",
                ))
                .padding(Padding::from([8, 12]));
                if shareable {
                    base.style(theme::secondary_button)
                        .on_press(Message::ShareContact(num))
                } else {
                    base.style(theme::card_button_disabled)
                }
            },
            button(widgets::labelled(
                if node.is_ignored {
                    lucide::bell().size(14).color(theme::text())
                } else {
                    lucide::bell_off().size(14).color(theme::text())
                },
                if node.is_ignored { "Unmute" } else { "Mute" },
            ))
            .padding(Padding::from([8, 12]))
            .style(theme::secondary_button)
            .on_press(Message::ToggleIgnored(num)),
            Space::new().width(Length::Fill),
            button(widgets::labelled(
                lucide::trash_two().size(14).color(theme::danger()),
                "Remove",
            ))
            .padding(Padding::from([8, 12]))
            .style(theme::danger_button)
            .on_press(Message::RemoveNode(num)),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    container(
        scrollable(container(body).padding(Padding::from(22)))
            .height(Length::Fill)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::content)
    .into()
}

fn position_block<'a>(
    app: &'a crate::app::App,
    position: Option<&'a Position>,
) -> Element<'a, Message> {
    let Some(position) = position else {
        return column![
            text("POSITION").size(11).color(theme::text_faint()),
            text("No position reported")
                .size(13)
                .color(theme::text_muted()),
        ]
        .spacing(4)
        .into();
    };

    let (Some(lat), Some(lon)) = (position.latitude_i, position.longitude_i) else {
        return column![
            text("POSITION").size(11).color(theme::text_faint()),
            text("Position without coordinates")
                .size(13)
                .color(theme::text_muted()),
        ]
        .spacing(4)
        .into();
    };

    let coords = format!("{:.5}, {:.5}", lat as f64 * 1e-7, lon as f64 * 1e-7);
    let distance = app
        .my_node_num
        .and_then(|me| app.node(me))
        .and_then(|me| me.position.as_ref())
        .and_then(|p| Some((p.latitude_i?, p.longitude_i?)))
        .map(|mine| {
            format::format_distance(format::distance_km((lat, lon), mine), app.settings.imperial)
        });

    column![
        text("POSITION").size(11).color(theme::text_faint()),
        row![
            widgets::stat("Coordinates", coords),
            widgets::stat(
                "Altitude",
                position
                    .altitude
                    .map(|a| format!("{a} m"))
                    .unwrap_or_else(|| "-".into())
            ),
            widgets::stat("Age", format::relative_time(position.time, app.now)),
            widgets::stat("Distance", distance.unwrap_or_else(|| "-".into())),
        ]
        .spacing(16),
    ]
    .spacing(8)
    .into()
}

/// Environment, air-quality and power telemetry, if a sample carries any.
///
/// Device metrics are deliberately excluded: they are already shown in the
/// dedicated row above, and repeating them here was confusing. Returns `None`
/// when a sample has nothing new to add, so the panel stays free of duplicate
/// or empty sections.
fn telemetry_block(
    sample: &meshtastic_protobufs::meshtastic::Telemetry,
) -> Option<Element<'_, Message>> {
    let mut stats = row![].spacing(16);
    match &sample.variant {
        Some(telemetry::Variant::EnvironmentMetrics(m)) => {
            stats = stats.push(widgets::stat(
                "Temperature",
                m.temperature
                    .map(|v| format!("{v:.1} °C"))
                    .unwrap_or_else(|| "-".into()),
            ));
            stats = stats.push(widgets::stat(
                "Humidity",
                m.relative_humidity
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or_else(|| "-".into()),
            ));
            stats = stats.push(widgets::stat(
                "Pressure",
                m.barometric_pressure
                    .map(|v| format!("{v:.0} hPa"))
                    .unwrap_or_else(|| "-".into()),
            ));
        }
        Some(telemetry::Variant::AirQualityMetrics(m)) => {
            stats = stats.push(widgets::stat(
                "PM2.5",
                m.pm25_standard
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "-".into()),
            ));
        }
        Some(telemetry::Variant::PowerMetrics(m)) => {
            stats = stats.push(widgets::stat(
                "Ch1 voltage",
                m.ch1_voltage
                    .map(|v| format!("{v:.2} V"))
                    .unwrap_or_else(|| "-".into()),
            ));
        }
        // Local stats overlap with device metrics except for mesh bookkeeping,
        // so show only the parts that are not already on screen.
        Some(telemetry::Variant::LocalStats(m)) => {
            stats = stats.push(widgets::stat(
                "Nodes online",
                m.num_online_nodes.to_string(),
            ));
            stats = stats.push(widgets::stat("Packets RX", m.num_packets_rx.to_string()));
            stats = stats.push(widgets::stat("Packets TX", m.num_packets_tx.to_string()));
        }
        // Device metrics are shown above; other variants add nothing here.
        _ => return None,
    }

    Some(
        column![text("TELEMETRY").size(11).color(theme::text_faint()), stats,]
            .spacing(8)
            .into(),
    )
}

fn traceroute_block<'a>(app: &'a crate::app::App, num: u32) -> Element<'a, Message> {
    let pending = app
        .traceroute_pending
        .filter(|pending| pending.target == num);

    let mut header = row![text("TRACEROUTE").size(11).color(theme::text_faint())]
        .spacing(6)
        .align_y(Alignment::Center);

    let Some(trace) = app.traceroutes.get(&num) else {
        return column![
            header,
            text(if pending.is_some() {
                "Waiting for the device to trace the route…"
            } else {
                "No traceroute yet. Run one to see the route to this node."
            })
            .size(13)
            .color(theme::text_muted()),
        ]
        .spacing(8)
        .into();
    };

    if pending.is_some() {
        header = header.push(lucide::loader().size(11).color(theme::primary()));
        header = header.push(text("tracing again…").size(11).color(theme::text_muted()));
    } else {
        header = header.push(Space::new().width(Length::Fill));
        header = header.push(
            text(format::relative_time(trace.at as u32, app.now))
                .size(11)
                .color(theme::text_faint()),
        );
    }

    let mut block = column![header].spacing(10);
    if trace.route.len() > 1 {
        block = block.push(route_lines(
            app,
            "TOWARD DESTINATION",
            &trace.route,
            &trace.snr_towards,
            num,
        ));
    }
    if trace.route_back.len() > 1 {
        block = block.push(route_lines(
            app,
            "BACK TO US",
            &trace.route_back,
            &trace.snr_back,
            num,
        ));
    }

    block.into()
}

/// One direction of a traceroute: hops with the SNR of each link between.
fn route_lines<'a>(
    app: &'a crate::app::App,
    label: &'static str,
    hops: &'a [u32],
    snrs: &'a [i32],
    target: u32,
) -> Element<'a, Message> {
    let mut lines = column![text(label).size(10).color(theme::text_faint())].spacing(5);
    for (index, hop) in hops.iter().enumerate() {
        lines = lines.push(hop_row(app, *hop, target));
        if index + 1 < hops.len() {
            lines = lines.push(link_snr(snrs.get(index).copied()));
        }
    }

    container(lines)
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 4.0,
        })
        .into()
}

fn hop_row<'a>(app: &'a crate::app::App, num: u32, target: u32) -> Element<'a, Message> {
    let unknown = num == u32::MAX;
    let name = if unknown {
        "unknown hop".to_string()
    } else {
        app.node(num)
            .map(format::node_name)
            .unwrap_or_else(|| format::node_id(num))
    };

    let mut row = row![
        lucide::circle().size(7).color(theme::primary()),
        text(name).size(13).color(theme::text()),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    if !unknown && Some(num) == app.my_node_num {
        row = row.push(widgets::tag("you", theme::text_muted()));
    } else if !unknown && num == target {
        row = row.push(widgets::tag("target", theme::primary()));
    }

    row.into()
}

/// The SNR of the link between two hops; `-128` means the device did not
/// measure it (or the hop count did not add up).
fn link_snr(snr: Option<i32>) -> Element<'static, Message> {
    let label = match snr {
        Some(value) if value > -128 => format!("⇊ {:.1} dB", value as f32 / 4.0),
        _ => "⇊ ? dB".to_string(),
    };
    text(label).size(11).color(theme::text_muted()).into()
}

fn format_uptime(seconds: u32) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn non_empty(value: Option<&str>) -> String {
    match value {
        Some(value) if !value.trim().is_empty() => value.to_string(),
        _ => "-".to_string(),
    }
}

/// Whether a public key is present and not the all-zero "mismatch" sentinel.
fn has_public_key(key: &[u8]) -> bool {
    !key.is_empty() && key.iter().any(|byte| *byte != 0)
}

fn security_block(
    user: Option<&User>,
    verified: bool,
    is_local: bool,
) -> Element<'static, Message> {
    let security = crate::security::NodeSecurity::of(user, verified, is_local);

    column![
        text("SECURITY").size(11).color(theme::text_faint()),
        row![
            security.icon(15.0),
            text(security.label()).size(14).color(security.color()),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        text(security.detail()).size(12).color(theme::text_muted()),
    ]
    .spacing(6)
    .into()
}

fn public_key_block(key: &[u8]) -> Element<'static, Message> {
    let encoded = format::public_key_label(key).unwrap_or_default();
    let shown = if encoded.len() > 32 {
        format!("{}\n{}", &encoded[..32], &encoded[32..])
    } else {
        encoded.clone()
    };

    column![
        text("PUBLIC KEY").size(11).color(theme::text_faint()),
        row![
            text(shown)
                .size(11)
                .font(iced::Font::MONOSPACE)
                .color(theme::text())
                .width(Length::Fill),
            button(lucide::copy().size(12).color(theme::text_faint()))
                .padding(Padding::from([2, 4]))
                .style(theme::ghost_button)
                .on_press(Message::CopyText(encoded)),
        ]
        .spacing(8)
        .align_y(Alignment::Start),
    ]
    .spacing(6)
    .into()
}

fn battery_color(level: u32) -> iced::Color {
    match level {
        0..=15 => theme::danger(),
        16..=35 => theme::warning(),
        _ => theme::primary(),
    }
}

fn signal_color(snr: f32) -> iced::Color {
    match format::snr_bars(snr) {
        4 | 3 => theme::primary(),
        2 => theme::warning(),
        1 => theme::warning(),
        _ => theme::text_faint(),
    }
}
