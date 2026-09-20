//! Messages view: channel timelines and direct conversations.

use iced::widget::{Space, button, column, container, markdown, row, scrollable, text, text_input};
use iced::{Alignment, Border, Color, Element, Length, Padding, Theme};

use crate::app::{App, Conversation, Message};
use crate::format;
use crate::icons::lucide;
use crate::security;
use crate::theme;
use crate::widgets;
use mt_persistence::{MessageRecord, MessageStatus};

/// Widget id for the message composer, so a shortcut can focus it.
pub const COMPOSE_INPUT_ID: &str = "message-compose";

pub fn view(app: &App) -> Element<'_, Message> {
    row![sidebar(app), conversation_pane(app)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// Sidebar

fn sidebar(app: &App) -> Element<'_, Message> {
    let mut list = column![].spacing(2).width(Length::Fill);

    list = list.push(section_label("CHANNELS"));
    for channel in app.active_channels() {
        let index = channel.index.max(0) as u32;
        let name = format::channel_name(channel.settings.as_ref().map(|s| s.name.as_str()), index);
        let selected = app.conversation == Conversation::Channel(index);
        list = list.push(conversation_item(
            app,
            channel_icon(selected),
            name,
            Conversation::Channel(index),
            selected,
            Message::SelectChannel(index),
            None,
        ));
    }

    let peers = app.peers();
    if !peers.is_empty() {
        list = list.push(Space::new().height(10));
        list = list.push(section_label("DIRECT MESSAGES"));
        for peer in peers {
            let selected = app.conversation == Conversation::Peer(peer);
            let node = app.node(peer);
            let badge = security::NodeSecurity::of(
                node.and_then(|n| n.user.as_ref()),
                node.map(|n| n.is_key_manually_verified).unwrap_or(false),
                Some(peer) == app.my_node_num,
            )
            .icon(12.0);
            list = list.push(conversation_item(
                app,
                peer_icon(selected),
                app.node_name(peer),
                Conversation::Peer(peer),
                selected,
                Message::SelectPeer(peer),
                Some(badge),
            ));
        }
    }

    container(scrollable(list.padding(Padding::from(10))).height(Length::Fill))
        .width(268)
        .height(Length::Fill)
        .style(theme::panel)
        .into()
}

fn section_label(title: &str) -> Element<'_, Message> {
    container(text(title).size(11).color(theme::text_faint()))
        .width(Length::Fill)
        .padding(Padding::from([6, 8]))
        .into()
}

fn channel_icon(selected: bool) -> Element<'static, Message> {
    let color = if selected {
        theme::primary()
    } else {
        theme::text_faint()
    };
    lucide::hash().size(16).color(color).into()
}

fn peer_icon(selected: bool) -> Element<'static, Message> {
    let color = if selected {
        theme::primary()
    } else {
        theme::text_faint()
    };
    lucide::user_round().size(16).color(color).into()
}

/// One conversation row: icon, name, last-message preview and its time.
fn conversation_item(
    app: &App,
    icon: Element<'static, Message>,
    title: String,
    conversation: Conversation,
    selected: bool,
    on_press: Message,
    badge: Option<Element<'static, Message>>,
) -> Element<'static, Message> {
    let (preview, time) = match app.latest_message(conversation) {
        Some(record) => (
            preview_text(record),
            format::last_heard_short(record.sent_at.max(0) as u32, app.now),
        ),
        None => ("No messages".to_string(), String::new()),
    };

    let mut title_row = row![text(title).size(14).color(theme::text())]
        .spacing(6)
        .align_y(Alignment::Center);
    if let Some(badge) = badge {
        title_row = title_row.push(badge);
    }
    title_row = title_row
        .push(Space::new().width(Length::Fill))
        .push(text(time).size(10).color(theme::text_faint()));

    button(
        row![
            container(icon)
                .width(Length::Fixed(22.0))
                .center_x(Length::Fixed(22.0)),
            column![title_row, text(preview).size(11).color(theme::text_muted()),]
                .width(Length::Fill)
                .spacing(1),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([8, 10]))
    .style(theme::nav_button(selected))
    .on_press(on_press)
    .into()
}

/// A one-line preview of a message for the sidebar.
fn preview_text(record: &MessageRecord) -> String {
    let body = if record.text.trim().is_empty() {
        format!("[{}]", format::portnum_label(record.portnum))
    } else {
        record.text.replace(['\n', '\r'], " ")
    };
    let body = if record.outgoing {
        format!("You: {body}")
    } else {
        body
    };
    truncate(&body, 30)
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

// Conversation

fn conversation_pane(app: &App) -> Element<'_, Message> {
    container(
        column![
            conversation_header(app),
            widgets::divider(),
            timeline(app),
            widgets::divider(),
            compose(app),
        ]
        .height(Length::Fill)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::content)
    .into()
}

fn conversation_header(app: &App) -> Element<'_, Message> {
    let mut trailing = row![].spacing(8).align_y(Alignment::Center);

    let (avatar, title, subtitle) = match app.conversation {
        Conversation::Channel(index) => {
            let custom = app
                .channels
                .iter()
                .find(|c| c.index as u32 == index)
                .and_then(|c| c.settings.as_ref())
                .map(|s| s.name.as_str());
            let name = format::channel_name(custom, index);
            let role = if index == 0 {
                match app.modem_preset() {
                    Some(preset) => {
                        format!("Primary · {}", format::modem_preset_label(preset))
                    }
                    None => "Primary channel".to_string(),
                }
            } else {
                format!("Channel {index}")
            };
            trailing = trailing.push(security::ChannelSecurity::of(channel_psk(app, index)).chip());
            (channel_avatar(), name, format!("{role} · broadcast"))
        }
        Conversation::Peer(peer) => {
            let node = app.node(peer);
            let chip = security::NodeSecurity::of(
                node.and_then(|n| n.user.as_ref()),
                node.map(|n| n.is_key_manually_verified).unwrap_or(false),
                Some(peer) == app.my_node_num,
            )
            .chip();
            let subtitle = match node {
                Some(node) => format!(
                    "{} · heard {}",
                    format::node_id(peer),
                    format::relative_time(node.last_heard, app.now)
                ),
                None => format!("Direct · {}", format::node_id(peer)),
            };
            trailing = trailing.push(chip);
            trailing = trailing.push(
                button(widgets::labelled(
                    lucide::info().size(13).color(theme::text()),
                    "Details",
                ))
                .padding(Padding::from([6, 10]))
                .style(theme::secondary_button)
                .on_press(Message::OpenNodeDetails(peer)),
            );
            (node_avatar(app, peer), app.node_name(peer), subtitle)
        }
    };

    container(
        row![
            avatar,
            column![
                text(title).size(17).color(theme::text()),
                text(subtitle).size(12).color(theme::text_muted()),
            ]
            .width(Length::Fill)
            .spacing(2),
            trailing,
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([12, 18]))
    .into()
}

/// The raw pre-shared key of a channel, empty when unknown.
fn channel_psk(app: &App, index: u32) -> &[u8] {
    app.channels
        .iter()
        .find(|c| c.index as u32 == index)
        .and_then(|c| c.settings.as_ref())
        .map(|s| s.psk.as_slice())
        .unwrap_or(&[])
}

fn timeline(app: &App) -> Element<'_, Message> {
    let messages = app.conversation_messages();
    if messages.is_empty() {
        let (title, hint) = match app.conversation {
            Conversation::Channel(_) => (
                "No messages yet",
                "Messages sent to this channel will appear here.",
            ),
            Conversation::Peer(_) => (
                "No direct messages",
                "Say hello - messages are queued and sent when the link is up.",
            ),
        };
        return widgets::empty_state(
            lucide::message_square()
                .size(42)
                .color(theme::text_faint())
                .into(),
            title,
            hint,
        );
    }

    let mut list = column![].spacing(10).width(Length::Fill);
    let mut last_day = String::new();
    for record in messages {
        let day = format::day_label(record.sent_at, app.now);
        if !day.is_empty() && day != last_day {
            list = list.push(
                container(day_separator(day.clone()))
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            );
            last_day = day;
        }
        list = list.push(message_bubble(app, record));
    }

    scrollable(
        container(list)
            .width(Length::Fill)
            .padding(Padding::from([12, 18])),
    )
    .height(Length::Fill)
    .width(Length::Fill)
    .anchor_bottom()
    .into()
}

fn day_separator(label: String) -> Element<'static, Message> {
    container(text(label).size(10).color(theme::text_faint()))
        .padding(Padding::from([3, 10]))
        .style(|_| iced::widget::container::Style {
            background: Some(theme::surface_alt().into()),
            border: Border {
                color: theme::border(),
                width: 1.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn message_bubble<'a>(app: &'a App, record: &'a MessageRecord) -> Element<'a, Message> {
    let outgoing = record.outgoing;
    let body = if record.text.is_empty() {
        format!("[{}]", format::portnum_label(record.portnum))
    } else {
        record.text.clone()
    };

    let body_element: Element<'a, Message> = if record.text.is_empty() {
        text(body.clone()).size(14).color(theme::text()).into()
    } else if let Some(content) = app.markdown.get(&record.id) {
        markdown::view(
            content.items(),
            markdown::Settings::with_text_size(14.0, markdown::Style::from(&app.theme())),
        )
        .map(Message::OpenLink)
    } else {
        text(body.clone()).size(14).color(theme::text()).into()
    };

    let mut meta = row![
        text(format::clock_time(record.sent_at))
            .size(10)
            .color(theme::text_faint())
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    if outgoing {
        let color = status_color(record.status);
        meta = meta.push(status_mark(record.status));
        meta = meta.push(
            text(format::status_label(record.status))
                .size(10)
                .color(color),
        );
    }

    if let Some(error) = &record.error {
        meta = meta.push(text(error).size(10).color(theme::danger()));
    }

    meta = meta.push(
        button(lucide::copy().size(12).color(theme::text_faint()))
            .padding(Padding::from([2, 4]))
            .style(theme::ghost_button)
            .on_press(Message::CopyText(body.clone())),
    );

    let bubble = container(column![body_element, meta].spacing(5).align_x(if outgoing {
        Alignment::End
    } else {
        Alignment::Start
    }))
    .max_width(520.0)
    .padding(Padding::from([9, 12]))
    .style(bubble_style(outgoing));

    if outgoing {
        return row![Space::new().width(Length::Fill), bubble]
            .width(Length::Fill)
            .into();
    }

    // Incoming: show the sender's avatar, and their name on channels.
    let mut body = column![].spacing(3).align_x(Alignment::Start);
    if matches!(app.conversation, Conversation::Channel(_)) {
        body = body.push(
            text(app.node_name(record.from))
                .size(11)
                .color(theme::primary_dim()),
        );
    }
    body = body.push(bubble);

    row![
        node_avatar(app, record.from),
        body,
        Space::new().width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::End)
    .into()
}

fn status_color(status: MessageStatus) -> Color {
    match status {
        MessageStatus::Delivered => theme::primary(),
        MessageStatus::Enroute => theme::warning(),
        MessageStatus::Queued => theme::text_faint(),
        MessageStatus::Failed => theme::danger(),
    }
}

fn status_mark(status: MessageStatus) -> Element<'static, Message> {
    let color = status_color(status);
    match status {
        MessageStatus::Delivered => lucide::check_check().size(13).color(color).into(),
        MessageStatus::Enroute => lucide::loader().size(13).color(color).into(),
        MessageStatus::Queued => lucide::clock_three().size(12).color(color).into(),
        MessageStatus::Failed => lucide::circle_alert().size(13).color(color).into(),
    }
}

fn bubble_style(outgoing: bool) -> impl Fn(&Theme) -> iced::widget::container::Style {
    move |_| iced::widget::container::Style {
        background: Some(if outgoing {
            Color::from_rgba(
                theme::primary().r,
                theme::primary().g,
                theme::primary().b,
                0.13,
            )
            .into()
        } else {
            theme::surface_alt().into()
        }),
        text_color: Some(theme::text()),
        border: Border {
            color: if outgoing {
                theme::primary_dim()
            } else {
                theme::border()
            },
            width: 1.0,
            radius: 12.0.into(),
        },
        ..Default::default()
    }
}

// Avatars

fn channel_avatar() -> Element<'static, Message> {
    avatar_circle(
        lucide::hash().size(15).color(theme::primary()).into(),
        theme::primary_dim(),
    )
}

fn node_avatar(app: &App, num: u32) -> Element<'static, Message> {
    let name = app
        .node(num)
        .map(format::node_short_name)
        .unwrap_or_else(|| "?".to_string());
    let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
    avatar_circle(
        text(initials).size(11).color(Color::WHITE).into(),
        avatar_color(num),
    )
}

fn avatar_circle(
    content: Element<'static, Message>,
    background: Color,
) -> Element<'static, Message> {
    container(content)
        .width(Length::Fixed(32.0))
        .height(Length::Fixed(32.0))
        .center_x(Length::Fixed(32.0))
        .center_y(Length::Fixed(32.0))
        .style(move |_| iced::widget::container::Style {
            background: Some(background.into()),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Avatar tints now live in the theme so the map markers can reuse them.
fn avatar_color(num: u32) -> Color {
    theme::avatar_color(num)
}

// Compose

fn compose(app: &App) -> Element<'_, Message> {
    let bytes = app.compose.trim().len();
    let over = bytes > crate::app::MAX_MESSAGE_BYTES;
    let can_send = !app.compose.trim().is_empty() && !over;
    let count_color = if over {
        theme::danger()
    } else if bytes > 0 {
        theme::text_muted()
    } else {
        theme::text_faint()
    };
    let hint = if app.settings.send_on_enter {
        "Enter to send"
    } else {
        "Ctrl+Enter to send"
    };

    let send_icon = if can_send {
        lucide::send_horizontal()
            .size(14)
            .color(Color::from_rgb8(9, 20, 14))
    } else {
        lucide::send_horizontal()
            .size(14)
            .color(theme::text_faint())
    };
    let send = if can_send {
        button(widgets::labelled(send_icon, "Send"))
            .padding(Padding::from([11, 20]))
            .style(theme::primary_button)
            .on_press(Message::SendPressed)
    } else {
        button(widgets::labelled(send_icon, "Send"))
            .padding(Padding::from([11, 20]))
            .style(theme::ghost_button)
    };

    let mut input = text_input("Type a message…", &app.compose)
        .id(COMPOSE_INPUT_ID)
        .on_input(Message::ComposeChanged)
        .style(theme::text_input_style)
        .padding(Padding::from([11, 14]))
        .size(14)
        .width(Length::Fill);
    if app.settings.send_on_enter {
        input = input.on_submit(Message::SendPressed);
    }

    container(
        column![
            row![
                text(hint).size(10).color(theme::text_faint()),
                Space::new().width(Length::Fill),
                text(format!("{bytes}/{}", crate::app::MAX_MESSAGE_BYTES))
                    .size(10)
                    .color(count_color),
            ]
            .align_y(Alignment::Center),
            row![input, send].spacing(8).align_y(Alignment::Center),
        ]
        .spacing(6)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(Padding::from([10, 18]))
    .into()
}
