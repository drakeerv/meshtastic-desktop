//! BLE pairing dialog: enter the passkey shown on the device's screen.

use iced::widget::{Space, button, column, row, text, text_input};
use iced::{Alignment, Element, Padding};

use crate::app::{App, BlePairing, Message};
use crate::icons::lucide;
use crate::theme;
use crate::views::dialog::{card, scrim};

/// Widget id for the passkey field, so it can be focused automatically.
pub const PASSKEY_INPUT_ID: &str = "meshtastic-ble-passkey";

/// The pairing overlay, if a device is asking for its passkey.
pub fn overlay(app: &App) -> Option<Element<'_, Message>> {
    app.ble_pairing.as_ref().map(dialog)
}

fn dialog(pairing: &BlePairing) -> Element<'_, Message> {
    let can_submit = pairing.input.trim().len() >= 6;
    let name = pairing
        .address
        .strip_prefix("x")
        .unwrap_or(&pairing.address);

    let submit = {
        let base = button(crate::widgets::labelled(
            lucide::bluetooth().size(14).color(if can_submit {
                iced::Color::from_rgb8(9, 20, 14)
            } else {
                theme::text_faint()
            }),
            "Pair",
        ))
        .padding(Padding::from([8, 14]));
        if can_submit {
            base.style(theme::primary_button)
                .on_press(Message::SubmitBlePasskey)
        } else {
            base.style(theme::card_button_disabled)
        }
    };

    scrim(card(
        column![
            row![
                lucide::bluetooth().size(18).color(theme::primary()),
                text("Pair Bluetooth device").size(17).color(theme::text()),
                Space::new().width(iced::Length::Fill),
                button(lucide::x().size(15).color(theme::text_muted()))
                    .padding(Padding::from([4, 8]))
                    .style(theme::ghost_button)
                    .on_press(Message::DismissBlePairing),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(format!("{name} is showing a 6-digit passkey on its screen."))
                .size(13)
                .color(theme::text()),
            text("Read it first, then type it here. Each attempt allows about 20 seconds; we retry automatically until it sticks.")
                .size(12)
                .color(theme::text_muted()),
            text_input("123456", &pairing.input)
                .id(PASSKEY_INPUT_ID)
                .on_input(Message::BlePasskeyChanged)
                .on_submit(Message::SubmitBlePasskey)
                .style(theme::text_input_style)
                .padding(Padding::from([10, 12]))
                .size(20),
            row![
                Space::new().width(iced::Length::Fill),
                button(text("Cancel").size(13))
                    .padding(Padding::from([8, 14]))
                    .style(theme::secondary_button)
                    .on_press(Message::DismissBlePairing),
                submit,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(14),
    ))
}
