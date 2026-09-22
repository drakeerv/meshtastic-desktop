//! The left navigation rail.

use iced::widget::{Space, button, column, container, text};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use crate::app::{App, Message, Tab};
use crate::icons::lucide;
use crate::theme;

/// The height of every cell in the rail. The logo and the tab buttons share
/// it so the column has an even rhythm.
const NAV_ITEM_HEIGHT: f32 = 56.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let mut items = column![].spacing(6).align_x(Alignment::Center);

    items = items.push(
        container(
            iced::widget::Svg::new(crate::assets::logo(app.is_light()))
                .width(34)
                .height(19),
        )
        .width(Length::Fill)
        .height(Length::Fixed(NAV_ITEM_HEIGHT))
        .center_x(Length::Fill)
        .center_y(Length::Fixed(NAV_ITEM_HEIGHT)),
    );

    for tab in Tab::ALL {
        items = items.push(nav_item(app, tab));
    }

    container(
        column![
            items,
            Space::new().height(Length::Fill),
            connection_indicator(app)
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center),
    )
    .width(76)
    .height(Length::Fill)
    .padding(Padding::from([14, 8]))
    .style(theme::rail)
    .into()
}

fn nav_item<'a>(app: &App, tab: Tab) -> Element<'a, Message> {
    let active = app.tab == tab;
    let glyph = match tab {
        Tab::Messages => lucide::message_square(),
        Tab::Nodes => lucide::radio_tower(),
        Tab::Map => lucide::map(),
        // Connect covers Bluetooth, USB serial and WiFi alike, so avoid a
        // transport-specific glyph here.
        Tab::Connect => lucide::plug(),
        Tab::Settings => lucide::settings(),
    };
    let color = if active {
        theme::primary()
    } else {
        theme::text_muted()
    };

    let mut icon: Element<'static, Message> = container(glyph.size(19).color(color))
        .width(Length::Fill)
        .height(Length::Fixed(22.0))
        .center_x(Length::Fill)
        .center_y(Length::Fixed(22.0))
        .into();

    let unread = if tab == Tab::Messages {
        app.total_unread()
    } else {
        0
    };
    if unread > 0 {
        let badge = container(
            text(unread.min(99).to_string())
                .size(9)
                .color(Color::from_rgb8(9, 20, 14)),
        )
        .padding(Padding::from([0, 4]))
        .style(|_: &iced::Theme| iced::widget::container::Style {
            background: Some(theme::primary().into()),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        });

        icon = iced::widget::stack![
            icon,
            container(badge)
                .width(Length::Fill)
                .height(Length::Fixed(22.0))
                .align_x(Alignment::End)
                .align_y(Alignment::Start)
                .padding(Padding::from([0, 12])),
        ]
        .width(Length::Fill)
        .height(Length::Fixed(22.0))
        .into();
    }

    button(
        column![
            icon,
            text(tab.label()).size(10).center().width(Length::Fill),
        ]
        .spacing(2)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(NAV_ITEM_HEIGHT))
    .padding(Padding::from([8, 2]))
    .style(theme::nav_button(active))
    .on_press(Message::SelectTab(tab))
    .into()
}

fn connection_indicator(app: &App) -> Element<'_, Message> {
    use mt_core::ConnectionState;
    let (color, label) = match &app.conn {
        ConnectionState::Disconnected => (theme::text_faint(), "offline"),
        ConnectionState::Connecting(_) => (theme::warning(), "connecting"),
        ConnectionState::Handshaking(_) => (theme::warning(), "syncing"),
        ConnectionState::Connected { .. } => (theme::primary(), "online"),
        ConnectionState::Reconnecting { .. } => (theme::warning(), "retrying"),
        ConnectionState::Disconnecting(_) => (theme::text_muted(), "closing"),
    };

    column![
        container(Space::new().width(10).height(10)).style(move |_: &iced::Theme| {
            iced::widget::container::Style {
                background: Some(color.into()),
                border: Border {
                    color,
                    width: 0.0,
                    radius: 999.0.into(),
                },
                ..Default::default()
            }
        }),
        text(label)
            .size(9)
            .color(color)
            .center()
            .width(Length::Fill),
    ]
    .spacing(4)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into()
}
