//! Rendering for the configuration and channel editors, plus the lists that
//! launch them from the Settings view.

use iced::widget::{Space, button, checkbox, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::{App, Message};
use crate::config_editor::{self, CHANNEL_ROLES, Field, FieldKind, SectionSpec};
use crate::format;
use crate::icons::lucide;
use crate::security;
use crate::theme;

// Section list (Settings page)

/// Rows for every configurable section, with an Edit action. When
/// `enabled` is false (no device connected) the rows are inert.
pub fn config_list(app: &App, enabled: bool) -> Element<'_, Message> {
    let mut list = column![].spacing(8).width(Length::Fill);
    for spec in config_editor::all_specs() {
        list = list.push(section_row(app, spec, enabled));
    }
    list.into()
}

fn section_row(app: &App, spec: &'static SectionSpec, enabled: bool) -> Element<'static, Message> {
    let configured = if spec.section.is_module() {
        app.module_configs
            .iter()
            .any(|c| config_editor::SectionValue::from_module_config(spec.section, c).is_some())
    } else {
        app.device_configs
            .iter()
            .any(|c| config_editor::SectionValue::from_config(spec.section, c).is_some())
    };

    let state = if configured {
        widgets_tag("configured", theme::primary())
    } else {
        widgets_tag("defaults", theme::text_faint())
    };

    container(
        row![
            column![
                text(spec.title).size(14).color(if enabled {
                    theme::text()
                } else {
                    theme::text_faint()
                }),
                text(spec.description).size(12).color(theme::text_muted()),
            ]
            .width(Length::Fill)
            .spacing(2),
            state,
            edit_button(enabled, Message::OpenConfigEditor(spec.section)),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding(Padding::from([10, 12]))
    .style(theme::inset)
    .into()
}

/// An Edit button that becomes inert when no device is connected.
fn edit_button(enabled: bool, message: Message) -> Element<'static, Message> {
    let style: fn(&iced::Theme, iced::widget::button::Status) -> iced::widget::button::Style =
        if enabled {
            theme::secondary_button
        } else {
            theme::ghost_button
        };
    let mut view = button(text("Edit").size(13))
        .padding(Padding::from([7, 14]))
        .style(style);
    if enabled {
        view = view.on_press(message);
    }
    view.into()
}

/// Rows for the eight channel slots.
pub fn channel_list(app: &App, enabled: bool) -> Element<'_, Message> {
    let mut list = column![].spacing(8).width(Length::Fill);
    for index in 0..8i32 {
        let channel = app.channels.iter().find(|c| c.index == index);
        let mut dim = !enabled;
        let (name, role, security) = match channel {
            Some(channel) => {
                let disabled = channel.role
                    == meshtastic_protobufs::meshtastic::channel::Role::Disabled as i32;
                dim = dim || disabled;
                let settings = channel.settings.as_ref();
                let name = if disabled {
                    format!("Slot {index}")
                } else {
                    format::channel_name(settings.map(|s| s.name.as_str()), index as u32)
                };
                let role = role_label(channel.role).to_string();
                let security = settings.map(|s| security::ChannelSecurity::of(&s.psk));
                (name, role, security)
            }
            None => (format!("Slot {index}"), "empty".to_string(), None),
        };

        let mut content = row![
            column![
                text(name).size(14).color(if dim {
                    theme::text_faint()
                } else {
                    theme::text()
                }),
                text(format!("slot {index} · {role}"))
                    .size(12)
                    .color(theme::text_muted()),
            ]
            .width(Length::Fill)
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        if let Some(security) = security {
            content = content.push(security.chip());
        }
        content = content.push(edit_button(enabled, Message::OpenChannelEditor(index)));

        list = list.push(
            container(content)
                .width(Length::Fill)
                .padding(Padding::from([10, 12]))
                .style(theme::inset),
        );
    }
    list.into()
}

fn role_label(role: i32) -> &'static str {
    CHANNEL_ROLES
        .iter()
        .find(|(_, value)| *value == role)
        .map(|(label, _)| *label)
        .unwrap_or("unknown")
}

// Config editor

pub fn editor_view(app: &App) -> Element<'_, Message> {
    let Some(editor) = &app.editor else {
        return crate::widgets::empty_state(
            lucide::settings()
                .size(42)
                .color(theme::text_faint())
                .into(),
            "No editor open",
            "Pick a section to edit.",
        );
    };

    let mut form = column![].spacing(14).width(Length::Fill);
    for field in editor.spec.fields {
        form = form.push(field_row(editor, field));
    }

    let footer = editor_footer(editor.error.as_deref(), false);
    let body = column![form.width(Length::Fill), footer]
        .spacing(18)
        .width(Length::Fill);

    crate::views::settings::page(
        crate::views::settings::header(
            editor.spec.title,
            editor.spec.description,
            Some(Message::CloseEditor),
        ),
        body,
    )
}

fn field_row<'a>(editor: &'a config_editor::Editor, field: &'static Field) -> Element<'a, Message> {
    row![
        column![
            text(field.label).size(13).color(theme::text()),
            text(field.help).size(11).color(theme::text_muted()),
        ]
        .width(Length::Fill)
        .spacing(2),
        field_control(editor, field),
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

fn field_control<'a>(
    editor: &'a config_editor::Editor,
    field: &'static Field,
) -> Element<'a, Message> {
    let key = field.key.to_string();
    match field.kind {
        FieldKind::Bool => {
            let checked = editor.value(field.key).eq_ignore_ascii_case("true");
            checkbox(checked)
                .on_toggle(move |value| Message::EditorFieldChanged {
                    key: key.clone(),
                    value: value.to_string(),
                })
                .into()
        }
        FieldKind::Enum(options) => {
            let choices: Vec<Choice> = options
                .iter()
                .map(|(label, value)| Choice {
                    label,
                    value: *value,
                })
                .collect();
            let selected = editor
                .value(field.key)
                .parse::<i32>()
                .ok()
                .and_then(|value| choices.iter().find(|c| c.value == value).cloned());
            pick_list(choices, selected, move |choice| {
                Message::EditorFieldChanged {
                    key: key.clone(),
                    value: choice.value.to_string(),
                }
            })
            .text_size(13)
            .padding(Padding::from([7, 10]))
            .width(Length::Fixed(240.0))
            .into()
        }
        FieldKind::Password => text_input("", editor.value(field.key))
            .secure(true)
            .on_input(move |value| Message::EditorFieldChanged {
                key: key.clone(),
                value,
            })
            .style(theme::text_input_style)
            .padding(Padding::from([8, 10]))
            .size(13)
            .width(Length::Fixed(280.0))
            .into(),
        _ => text_input("", editor.value(field.key))
            .on_input(move |value| Message::EditorFieldChanged {
                key: key.clone(),
                value,
            })
            .style(theme::text_input_style)
            .padding(Padding::from([8, 10]))
            .size(13)
            .width(Length::Fixed(280.0))
            .into(),
    }
}

// Channel editor

pub fn channel_editor_view(app: &App) -> Element<'_, Message> {
    let Some(editor) = &app.channel_editor else {
        return crate::widgets::empty_state(
            lucide::hash().size(42).color(theme::text_faint()).into(),
            "No channel open",
            "Pick a slot to edit.",
        );
    };

    let mut form = column![].spacing(16).width(Length::Fill);

    // Name
    form = form.push(text_row(
        "Name",
        "Shown in the channel list and broadcast as the channel name.",
        text_input("Primary", editor.value("name"))
            .on_input(|value| Message::EditorFieldChanged {
                key: "name".into(),
                value,
            })
            .style(theme::text_input_style)
            .padding(Padding::from([8, 10]))
            .size(13)
            .width(Length::Fixed(280.0)),
    ));

    // PSK
    let psk_row = row![
        text_input("hex key (empty = open)", editor.value("psk"))
            .on_input(|value| Message::EditorFieldChanged {
                key: "psk".into(),
                value,
            })
            .style(theme::text_input_style)
            .padding(Padding::from([8, 10]))
            .size(13)
            .width(Length::Fixed(230.0)),
        button(text("Default").size(12))
            .padding(Padding::from([7, 10]))
            .style(theme::secondary_button)
            .on_press(Message::EditorFieldChanged {
                key: "psk".into(),
                value: "01".into(),
            }),
        button(text("Open").size(12))
            .padding(Padding::from([7, 10]))
            .style(theme::secondary_button)
            .on_press(Message::EditorFieldChanged {
                key: "psk".into(),
                value: String::new(),
            }),
        button(text("Random").size(12))
            .padding(Padding::from([7, 10]))
            .style(theme::secondary_button)
            .on_press(Message::RandomizeChannelPsk),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    form = form.push(help_row(
        "Key",
        "Hex AES key: empty for open, 32 chars for 128-bit, 64 for 256-bit.",
        psk_row.into(),
    ));

    // Role
    let role_control: Element<'_, Message> = if editor.index == 0 {
        text("Primary (slot 0)")
            .size(13)
            .color(theme::text_muted())
            .into()
    } else {
        let choices: Vec<Choice> = CHANNEL_ROLES
            .iter()
            .map(|(label, value)| Choice {
                label,
                value: *value,
            })
            .collect();
        let selected = editor
            .value("role")
            .parse::<i32>()
            .ok()
            .and_then(|value| choices.iter().find(|c| c.value == value).cloned());
        pick_list(choices, selected, |choice| Message::EditorFieldChanged {
            key: "role".into(),
            value: choice.value.to_string(),
        })
        .text_size(13)
        .padding(Padding::from([7, 10]))
        .width(Length::Fixed(240.0))
        .into()
    };
    form = form.push(help_row(
        "Role",
        "Primary, secondary or disabled for this slot.",
        role_control,
    ));

    // Uplink / downlink
    form = form.push(help_row(
        "Uplink",
        "Forward packets from this channel to MQTT.",
        checkbox(editor.value("uplink_enabled").eq_ignore_ascii_case("true"))
            .on_toggle(|value| Message::EditorFieldChanged {
                key: "uplink_enabled".into(),
                value: value.to_string(),
            })
            .into(),
    ));
    form = form.push(help_row(
        "Downlink",
        "Accept packets for this channel from MQTT.",
        checkbox(
            editor
                .value("downlink_enabled")
                .eq_ignore_ascii_case("true"),
        )
        .on_toggle(|value| Message::EditorFieldChanged {
            key: "downlink_enabled".into(),
            value: value.to_string(),
        })
        .into(),
    ));

    let footer = editor_footer(editor.error.as_deref(), true);
    let body = column![form.width(Length::Fill), footer]
        .spacing(18)
        .width(Length::Fill);

    crate::views::settings::page(
        crate::views::settings::header(
            format!("Channel {}", editor.index),
            "Name, key and encryption settings for this slot.",
            Some(Message::CloseEditor),
        ),
        body,
    )
}

// Shared pieces

fn editor_footer(error: Option<&str>, channel: bool) -> Element<'static, Message> {
    let mut bar = row![Space::new().width(Length::Fill).height(Length::Shrink),]
        .spacing(10)
        .align_y(Alignment::Center);

    if let Some(error) = error {
        bar = bar.push(text(error.to_string()).size(12).color(theme::danger()));
    }

    bar = bar.push(
        button(text("Cancel").size(13))
            .padding(Padding::from([8, 16]))
            .style(theme::secondary_button)
            .on_press(Message::CloseEditor),
    );
    bar = bar.push(
        button(text(if channel { "Save channel" } else { "Save" }).size(13))
            .padding(Padding::from([8, 18]))
            .style(theme::primary_button)
            .on_press(Message::SaveEditor),
    );

    container(bar)
        .width(Length::Fill)
        .padding(Padding::from([12, 22]))
        .into()
}

fn text_row<'a>(
    label: &'a str,
    help: &'a str,
    control: iced::widget::TextInput<'a, Message, iced::Theme, iced::Renderer>,
) -> Element<'a, Message> {
    help_row(label, help, control.into())
}

fn help_row<'a>(
    label: &'a str,
    help: &'a str,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    row![
        column![
            text(label).size(13).color(theme::text()),
            text(help).size(11).color(theme::text_muted()),
        ]
        .width(Length::Fill)
        .spacing(2),
        control,
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

fn widgets_tag(label: &'static str, color: iced::Color) -> Element<'static, Message> {
    container(text(label).size(11).color(color))
        .padding(Padding::from([2, 8]))
        .style(theme::status_pill(color))
        .into()
}

/// A display value for enum pick lists.
#[derive(Debug, Clone, PartialEq)]
struct Choice {
    label: &'static str,
    value: i32,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label)
    }
}
