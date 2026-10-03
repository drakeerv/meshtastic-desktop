//! A compact node profile card, opened from any conversation as a modal.
//!
//! It is the quick, in-context counterpart to the full Nodes detail pane:
//! enough to recognise who is talking and start a DM, favourite them, or jump
//! to the complete details.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Message};
use crate::format;
use crate::icons::lucide;
use crate::security;
use crate::theme;
use crate::views::dialog;
use crate::widgets;

/// The quick profile card, if a node is selected for it.
pub fn overlay(app: &App) -> Option<Element<'_, Message>> {
    let num = app.profile_node?;
    Some(dialog::scrim(dialog::card(card(app, num))))
}

fn card(app: &App, num: u32) -> Element<'_, Message> {
    let Some(node) = app.node(num) else {
        return column![
            text("Node unavailable").size(17).color(theme::text()),
            text("This node is no longer in the database.")
                .size(12)
                .color(theme::text_muted()),
            row![
                Space::new().width(Length::Fill),
                button(text("Close").size(13))
                    .padding(Padding::from([8, 14]))
                    .style(theme::secondary_button)
                    .on_press(Message::CloseProfile),
            ],
        ]
        .spacing(14)
        .into();
    };

    let user = node.user.as_ref();
    let model = user.map(|u| u.hw_model).unwrap_or(0);
    let chip = security::NodeSecurity::of(
        user,
        node.is_key_manually_verified,
        Some(num) == app.my_node_num,
    )
    .chip();

    let subtitle = match user {
        Some(user) => format!(
            "{} · {} · heard {}",
            format::role_label(user.role),
            format::hw_model_label(user.hw_model),
            format::relative_time(node.last_heard, app.now),
        ),
        None => format!("heard {}", format::relative_time(node.last_heard, app.now)),
    };

    let identity = row![
        container(
            iced::widget::Svg::new(crate::assets::device_art(model))
                .width(Length::Fixed(64.0))
                .height(Length::Fixed(64.0))
                .content_fit(iced::ContentFit::Contain),
        )
        .width(Length::Fixed(64.0))
        .height(Length::Fixed(64.0))
        .center_x(Length::Fixed(64.0))
        .center_y(Length::Fixed(64.0)),
        column![
            row![
                text(format::node_name(node)).size(19).color(theme::text()),
                chip,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(format::node_id(num))
                .size(13)
                .color(theme::text_muted()),
            text(subtitle).size(12).color(theme::text_muted()),
        ]
        .spacing(3)
        .width(Length::Fill),
        button(lucide::x().size(15).color(theme::text_muted()))
            .padding(Padding::from([6, 8]))
            .style(theme::ghost_button)
            .on_press(Message::CloseProfile),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let short_name = user
        .map(|u| u.short_name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "-".to_string());
    let stats = row![
        widgets::stat("Short name", short_name),
        widgets::stat("SNR", format!("{:.1} dB", node.snr)),
        widgets::stat("Hops", format::hops_label(node.hops_away)),
    ]
    .spacing(16);

    let favourite = button(widgets::labelled(
        lucide::star().size(14).color(if node.is_favorite {
            theme::warning()
        } else {
            theme::text_muted()
        }),
        if node.is_favorite {
            "Favourited"
        } else {
            "Favourite"
        },
    ))
    .padding(Padding::from([7, 12]))
    .style(theme::secondary_button)
    .on_press(Message::ToggleFavorite(num));

    let actions = row![
        favourite,
        Space::new().width(Length::Fill),
        button(widgets::labelled(
            lucide::info().size(14).color(theme::text()),
            "Details",
        ))
        .padding(Padding::from([7, 12]))
        .style(theme::secondary_button)
        .on_press(Message::OpenNodeDetails(num)),
        button(widgets::labelled(
            lucide::message_circle()
                .size(14)
                .color(iced::Color::from_rgb8(9, 20, 14)),
            "Direct message",
        ))
        .padding(Padding::from([7, 12]))
        .style(theme::primary_button)
        .on_press(Message::OpenConversation(num)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    column![
        identity,
        widgets::divider(),
        stats,
        widgets::divider(),
        actions
    ]
    .spacing(14)
    .into()
}
