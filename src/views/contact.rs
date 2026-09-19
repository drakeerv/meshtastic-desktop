//! Contact sharing and import dialogs.
//!
//! These are rendered as a full-window overlay (a scrim plus a centred card)
//! on top of whichever section is active. Sharing shows the node's contact as
//! a QR code and a copyable link; importing decodes a pasted link before
//! sending it to the device.

use iced::widget::{Space, button, column, container, qr_code, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length, Padding, Theme};

use crate::app::{App, ContactImport, ContactQr, Message};
use crate::icons::lucide;
use crate::theme;
use crate::views::dialog::{card, scrim};
use crate::widgets;

/// The contact overlay, if either dialog is open.
pub fn overlay(app: &App) -> Option<Element<'_, Message>> {
    if let Some(qr) = app.contact_qr.as_ref() {
        return Some(scrim(share_dialog(qr)));
    }
    app.contact_import
        .as_ref()
        .map(|import| scrim(import_dialog(import, app.is_connected())))
}

/// Render a node's contact as a scannable QR code with a copyable link.
fn share_dialog(qr: &ContactQr) -> Element<'_, Message> {
    let code = qr_code::QRCode::new(&qr.data)
        .total_size(260.0)
        .style(qr_style);

    card(
        column![
            row![
                lucide::contact().size(18).color(theme::primary()),
                text(&qr.title).size(17).color(theme::text()),
                Space::new().width(Length::Fill),
                button(lucide::x().size(15).color(theme::text_muted()))
                    .padding(Padding::from([4, 8]))
                    .style(theme::ghost_button)
                    .on_press(Message::CloseContactShare),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text("Scan this code with another client to add the contact and share its public key.")
                .size(12)
                .color(theme::text_muted()),
            container(code)
                .padding(Padding::from(12))
                .style(|_: &Theme| container::Style {
                    background: Some(iced::Color::WHITE.into()),
                    border: iced::Border {
                        color: theme::border(),
                        width: 1.0,
                        radius: 10.0.into(),
                    },
                    ..Default::default()
                }),
            container(
                text(&qr.uri)
                    .size(11)
                    .font(iced::Font::MONOSPACE)
                    .color(theme::text())
                    .wrapping(iced::advanced::text::Wrapping::WordOrGlyph)
                    .width(Length::Fill)
            )
            .width(Length::Fill)
            .padding(Padding::from([10, 12]))
            .style(theme::inset),
            button(widgets::labelled(
                lucide::copy().size(14).color(theme::text()),
                "Copy link",
            ))
            .width(Length::Fill)
            .padding(Padding::from([8, 14]))
            .style(theme::secondary_button)
            .on_press(Message::CopyText(qr.uri.clone())),
        ]
        .spacing(14)
        .align_x(Alignment::Center),
    )
}

/// Paste-a-link contact import.
fn import_dialog(import: &ContactImport, connected: bool) -> Element<'_, Message> {
    let can_import = connected && import.contact.is_some();

    let mut body = column![
        row![
            lucide::contact().size(18).color(theme::primary()),
            text("Import contact").size(17).color(theme::text()),
            Space::new().width(Length::Fill),
            button(lucide::x().size(15).color(theme::text_muted()))
                .padding(Padding::from([4, 8]))
                .style(theme::ghost_button)
                .on_press(Message::CloseContactImport),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        text("Paste a Meshtastic contact link (https://meshtastic.org/v/#…) or a bare payload.")
            .size(12)
            .color(theme::text_muted()),
        text_input("https://meshtastic.org/v/#…", &import.input)
            .on_input(Message::ContactImportChanged)
            .style(theme::text_input_style)
            .padding(Padding::from([9, 12]))
            .size(12),
    ]
    .spacing(14);

    if let Some(error) = import.error.as_ref() {
        body = body.push(
            row![
                lucide::triangle_alert().size(13).color(theme::danger()),
                text(error)
                    .size(12)
                    .color(theme::danger())
                    .width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }

    if let Some(preview) = import.preview.as_ref() {
        body = body.push(
            container(
                column![
                    text("CONTACT").size(11).color(theme::text_faint()),
                    text(&preview.name).size(15).color(theme::text()),
                    text(&preview.id).size(12).color(theme::text_muted()),
                    row![
                        lucide::lock().size(12).color(theme::primary()),
                        text("Public key present, direct messages will be encrypted")
                            .size(11)
                            .color(theme::text_muted()),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                ]
                .spacing(6),
            )
            .width(Length::Fill)
            .padding(Padding::from([12, 14]))
            .style(theme::inset),
        );
    }

    if !connected {
        body = body.push(
            text("Connect to a device to import a contact.")
                .size(12)
                .color(theme::warning()),
        );
    }

    let import_button = {
        let base = button(widgets::labelled(
            lucide::download().size(14).color(if can_import {
                iced::Color::from_rgb8(9, 20, 14)
            } else {
                theme::text_faint()
            }),
            "Import",
        ))
        .padding(Padding::from([8, 14]));
        if can_import {
            base.style(theme::primary_button)
                .on_press(Message::SubmitContactImport)
        } else {
            base.style(theme::card_button_disabled)
        }
    };

    body = body.push(
        row![
            Space::new().width(Length::Fill),
            button(text("Cancel").size(13))
                .padding(Padding::from([8, 14]))
                .style(theme::secondary_button)
                .on_press(Message::CloseContactImport),
            import_button,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    // Keep long payloads reachable on short windows.
    card(scrollable(body).height(Length::Shrink))
}

/// Colours for the QR code: dark cells on a light background regardless of
/// the app theme, so every scanner can read it.
fn qr_style(_theme: &Theme) -> qr_code::Style {
    qr_code::Style {
        cell: iced::Color::from_rgb8(13, 19, 17),
        background: iced::Color::WHITE,
    }
}
