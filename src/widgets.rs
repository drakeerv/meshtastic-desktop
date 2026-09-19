//! Small reusable presentation pieces shared across views.

use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Color, Element, Length, Padding};

use crate::app::Message;
use crate::theme;

/// A page title with an optional subtitle and right-aligned actions.
pub fn page_header<'a>(
    title: &'a str,
    subtitle: Option<String>,
    actions: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut titles = column![text(title).size(22).color(theme::text())];
    if let Some(subtitle) = subtitle {
        titles = titles.push(text(subtitle).size(13).color(theme::text_muted()));
    }

    let mut bar = row![titles.width(Length::Fill)];
    if !actions.is_empty() {
        let mut actions_row = row![].spacing(8).align_y(Alignment::Center);
        for action in actions {
            actions_row = actions_row.push(action);
        }
        bar = bar.push(actions_row);
    }

    container(bar)
        .width(Length::Fill)
        .padding(Padding::from([18, 22]))
        .into()
}

/// A labelled statistic block, used in detail panels.
pub fn stat(label: &str, value: String) -> Element<'_, Message> {
    column![
        text(label.to_uppercase())
            .size(10)
            .color(theme::text_faint()),
        text(value).size(15).color(theme::text()),
    ]
    .spacing(2)
    .width(Length::Fill)
    .into()
}

/// A coloured pill tag holding an icon.
pub fn icon_tag(icon: Element<'static, Message>, color: Color) -> Element<'static, Message> {
    container(icon)
        .padding(Padding::from([2, 8]))
        .style(theme::status_pill(color))
        .into()
}

/// A coloured pill tag.
pub fn tag(label: impl Into<String>, color: Color) -> Element<'static, Message> {
    container(text(label.into()).size(11).color(color))
        .padding(Padding::from([2, 8]))
        .style(theme::status_pill(color))
        .into()
}

/// A thin horizontal divider.
pub fn divider<'a>() -> Element<'a, Message> {
    container(Space::new())
        .width(Length::Fill)
        .height(1)
        .style(|_: &iced::Theme| container::Style {
            background: Some(theme::border().into()),
            ..Default::default()
        })
        .into()
}

/// A button label: an icon followed by text.
pub fn labelled<'a, I: Into<Element<'a, Message>>>(
    icon: I,
    label: &'a str,
) -> Element<'a, Message> {
    row![icon.into(), text(label).size(13)]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
}

/// An empty-state block with an icon, title and hint.
pub fn empty_state(
    icon: Element<'static, Message>,
    title: impl Into<String>,
    hint: impl Into<String>,
) -> Element<'static, Message> {
    container(
        column![
            icon,
            text(title.into()).size(17).color(theme::text()),
            text(hint.into())
                .size(13)
                .color(theme::text_muted())
                .align_x(Alignment::Center),
        ]
        .spacing(8)
        .align_x(Alignment::Center)
        .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}
