//! Shared chrome for the modal dialogs: a dimmed scrim and a centred card.

use iced::widget::container;
use iced::{Element, Length, Padding, Theme};

use crate::app::Message;
use crate::theme;

/// A dimmed, centred backdrop for a dialog.
pub fn scrim<'a>(dialog: Element<'a, Message>) -> Element<'a, Message> {
    container(dialog)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_: &Theme| container::Style {
            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.55).into()),
            ..Default::default()
        })
        .into()
}

/// The card chrome shared by the dialogs.
pub fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content.into())
        .width(Length::Fixed(400.0))
        .padding(Padding::from(22))
        .style(theme::card)
        .into()
}
