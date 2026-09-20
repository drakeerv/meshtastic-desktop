//! Meshtastic desktop theme.
//!
//! Colours are resolved from the *active palette* rather than hardcoded, so
//! switching between dark and light themes restyles the entire interface
//! (backgrounds, borders, text and controls), not just the widgets that
//! inherit iced's built-in palette.
//!
//! Widget style functions receive `&Theme` and resolve colours from it. View
//! code (which does not have the theme) reads the frame-active palette via
//! the accessor functions; [`set_light_mode`] is called once per frame from
//! `App::view`.

use std::sync::atomic::{AtomicBool, Ordering};

use iced::border::Radius;
use iced::theme::Palette;
use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector};

/// The full set of interface colours for one mode.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    pub background: Color,
    pub rail: Color,
    pub surface: Color,
    pub surface_alt: Color,
    pub surface_hover: Color,
    pub border: Color,
    pub text: Color,
    pub text_muted: Color,
    pub text_faint: Color,
    pub primary: Color,
    pub primary_dim: Color,
    pub danger: Color,
    pub warning: Color,
}

pub const DARK: Colors = Colors {
    background: Color::from_rgb8(13, 19, 17),
    rail: Color::from_rgb8(17, 24, 21),
    surface: Color::from_rgb8(23, 31, 28),
    surface_alt: Color::from_rgb8(31, 41, 37),
    surface_hover: Color::from_rgb8(38, 50, 45),
    border: Color::from_rgb8(44, 57, 52),
    text: Color::from_rgb8(230, 237, 233),
    text_muted: Color::from_rgb8(137, 159, 150),
    text_faint: Color::from_rgb8(96, 114, 107),
    primary: Color::from_rgb8(103, 234, 148),
    primary_dim: Color::from_rgb8(47, 165, 98),
    danger: Color::from_rgb8(229, 83, 75),
    warning: Color::from_rgb8(224, 164, 88),
};

pub const LIGHT: Colors = Colors {
    background: Color::from_rgb8(244, 247, 245),
    rail: Color::from_rgb8(233, 239, 236),
    surface: Color::from_rgb8(255, 255, 255),
    surface_alt: Color::from_rgb8(240, 244, 242),
    surface_hover: Color::from_rgb8(228, 235, 232),
    border: Color::from_rgb8(206, 217, 212),
    text: Color::from_rgb8(24, 33, 30),
    text_muted: Color::from_rgb8(88, 105, 98),
    text_faint: Color::from_rgb8(133, 148, 141),
    primary: Color::from_rgb8(32, 138, 76),
    primary_dim: Color::from_rgb8(47, 165, 98),
    danger: Color::from_rgb8(196, 54, 48),
    warning: Color::from_rgb8(176, 118, 36),
};

/// Frame-active light/dark flag, set by `App::view`.
static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

/// Frame-active high-contrast flag, set by `App::view`.
static HIGH_CONTRAST: AtomicBool = AtomicBool::new(false);

/// Set the palette used by the colour accessor functions for this frame.
pub fn set_light_mode(light: bool) {
    LIGHT_MODE.store(light, Ordering::Relaxed);
}

/// Boost the contrast of borders and secondary text for this frame.
pub fn set_high_contrast(high_contrast: bool) {
    HIGH_CONTRAST.store(high_contrast, Ordering::Relaxed);
}

/// Whether high contrast is active for this frame.
pub fn high_contrast() -> bool {
    HIGH_CONTRAST.load(Ordering::Relaxed)
}

/// The palette for the current frame.
pub fn current() -> &'static Colors {
    if LIGHT_MODE.load(Ordering::Relaxed) {
        &LIGHT
    } else {
        &DARK
    }
}

/// The palette implied by an iced theme.
pub fn colors_for(theme: &Theme) -> &'static Colors {
    let background = theme.palette().background;
    if background.r + background.g + background.b > 1.6 {
        &LIGHT
    } else {
        &DARK
    }
}

// Accessors used by views (frame-active palette).
pub fn surface_alt() -> Color {
    current().surface_alt
}
pub fn border() -> Color {
    if high_contrast() {
        current().text_muted
    } else {
        current().border
    }
}
pub fn text() -> Color {
    current().text
}
pub fn text_muted() -> Color {
    if high_contrast() {
        current().text
    } else {
        current().text_muted
    }
}
pub fn text_faint() -> Color {
    if high_contrast() {
        current().text_muted
    } else {
        current().text_faint
    }
}
pub fn primary() -> Color {
    current().primary
}
pub fn primary_dim() -> Color {
    current().primary_dim
}
pub fn danger() -> Color {
    current().danger
}
pub fn warning() -> Color {
    current().warning
}

/// Tints for node avatars, chosen to read against both palettes.
const AVATAR_COLORS: [Color; 6] = [
    Color::from_rgb8(0x35, 0x7A, 0xB8),
    Color::from_rgb8(0x46, 0x8C, 0x66),
    Color::from_rgb8(0xB8, 0x7E, 0x3B),
    Color::from_rgb8(0x9B, 0x6B, 0xC4),
    Color::from_rgb8(0xB8, 0x5C, 0x6B),
    Color::from_rgb8(0x4F, 0x9A, 0xA8),
];

/// A stable avatar tint for a node number, shared by the message list and the
/// map markers so a node keeps the same colour everywhere.
pub fn avatar_color(num: u32) -> Color {
    let index = ((num ^ (num >> 16)) as usize) % AVATAR_COLORS.len();
    AVATAR_COLORS[index]
}

/// The text and icon colour to draw on top of [`primary`].
pub fn on_primary() -> Color {
    Color::from_rgb8(9, 20, 14)
}

// themes

/// Build the dark application theme.
pub fn app_theme() -> Theme {
    Theme::custom(
        "Meshtastic".to_string(),
        Palette {
            background: DARK.background,
            text: DARK.text,
            primary: DARK.primary,
            success: DARK.primary,
            warning: DARK.warning,
            danger: DARK.danger,
        },
    )
}

/// Build the light application theme.
pub fn light_theme() -> Theme {
    Theme::custom(
        "Meshtastic Light".to_string(),
        Palette {
            background: LIGHT.background,
            text: LIGHT.text,
            primary: LIGHT.primary,
            success: LIGHT.primary,
            warning: LIGHT.warning,
            danger: LIGHT.danger,
        },
    )
}

// shadows

fn soft_shadow(light: bool) -> Shadow {
    Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.0, if light { 0.08 } else { 0.25 }),
        offset: Vector::new(0.0, 2.0),
        blur_radius: if light { 4.0 } else { 8.0 },
    }
}

// container styles

pub fn content(theme: &Theme) -> container::Style {
    let c = colors_for(theme);
    container::Style {
        background: Some(c.background.into()),
        text_color: Some(c.text),
        ..Default::default()
    }
}

pub fn rail(theme: &Theme) -> container::Style {
    let c = colors_for(theme);
    container::Style {
        background: Some(c.rail.into()),
        text_color: Some(c.text),
        border: Border {
            color: c.border,
            width: 0.0,
            radius: Radius::from(0.0),
        },
        ..Default::default()
    }
}

pub fn card(theme: &Theme) -> container::Style {
    let c = colors_for(theme);
    container::Style {
        background: Some(c.surface.into()),
        text_color: Some(c.text),
        border: Border {
            color: c.border,
            width: 1.0,
            radius: Radius::from(10.0),
        },
        shadow: soft_shadow(is_light(c)),
        ..Default::default()
    }
}

/// Whether a palette is the light one (by background luminance).
fn is_light(c: &Colors) -> bool {
    c.background.r + c.background.g + c.background.b > 1.6
}

pub fn panel(theme: &Theme) -> container::Style {
    let c = colors_for(theme);
    container::Style {
        background: Some(c.surface.into()),
        text_color: Some(c.text),
        ..Default::default()
    }
}

pub fn inset(theme: &Theme) -> container::Style {
    let c = colors_for(theme);
    container::Style {
        background: Some(c.surface_alt.into()),
        text_color: Some(c.text),
        border: Border {
            color: c.border,
            width: 1.0,
            radius: Radius::from(8.0),
        },
        ..Default::default()
    }
}

// button styles

pub fn nav_button(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = colors_for(theme);
        let background = if active {
            Some(Color::from_rgba(c.primary.r, c.primary.g, c.primary.b, 0.14).into())
        } else {
            match status {
                button::Status::Hovered | button::Status::Pressed => Some(c.surface_alt.into()),
                _ => None,
            }
        };
        button::Style {
            background,
            text_color: if active { c.primary } else { c.text_muted },
            border: Border {
                color: if active {
                    c.primary
                } else {
                    Color::TRANSPARENT
                },
                width: if active { 1.0 } else { 0.0 },
                radius: Radius::from(10.0),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors_for(theme);
    let background = match status {
        button::Status::Hovered => Color {
            r: (c.primary.r + 0.06).min(1.0),
            g: (c.primary.g + 0.02).min(1.0),
            b: (c.primary.b + 0.06).min(1.0),
            a: 1.0,
        },
        button::Status::Pressed => c.primary_dim,
        button::Status::Disabled => c.surface_hover,
        _ => c.primary,
    };
    button::Style {
        background: Some(background.into()),
        text_color: if is_light(c) {
            c.surface
        } else {
            Color::from_rgb8(9, 20, 14)
        },
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: Radius::from(8.0),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors_for(theme);
    let background = match status {
        button::Status::Hovered => c.surface_hover,
        button::Status::Pressed => c.surface_alt,
        button::Status::Disabled => c.surface,
        _ => c.surface_alt,
    };
    button::Style {
        background: Some(background.into()),
        text_color: c.text,
        border: Border {
            color: c.border,
            width: 1.0,
            radius: Radius::from(8.0),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn danger_button(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors_for(theme);
    let background = match status {
        button::Status::Hovered => Color::from_rgba(c.danger.r, c.danger.g, c.danger.b, 0.18),
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(background.into()),
        text_color: c.danger,
        border: Border {
            color: c.danger,
            width: 1.0,
            radius: Radius::from(8.0),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors_for(theme);
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Some(c.surface_alt.into()),
        _ => None,
    };
    button::Style {
        background,
        text_color: c.text_muted,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: Radius::from(6.0),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// A large clickable settings category card.
pub fn card_button(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors_for(theme);
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => c.surface_hover,
        _ => c.surface,
    };
    button::Style {
        background: Some(background.into()),
        text_color: c.text,
        border: Border {
            color: c.border,
            width: 1.0,
            radius: Radius::from(12.0),
        },
        shadow: soft_shadow(is_light(c)),
        snap: false,
    }
}

/// The disabled variant of [`card_button`].
pub fn card_button_disabled(theme: &Theme, _: button::Status) -> button::Style {
    let c = colors_for(theme);
    button::Style {
        background: Some(c.background.into()),
        text_color: c.text_faint,
        border: Border {
            color: c.border,
            width: 1.0,
            radius: Radius::from(12.0),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

// input styles

pub fn text_input_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let c = colors_for(theme);
    let border = match status {
        text_input::Status::Focused { .. } => c.primary,
        text_input::Status::Hovered => c.surface_hover,
        _ => c.border,
    };
    text_input::Style {
        background: c.surface_alt.into(),
        border: Border {
            color: border,
            width: 1.0,
            radius: Radius::from(8.0),
        },
        icon: c.text_muted,
        placeholder: c.text_faint,
        value: c.text,
        selection: Color::from_rgba(c.primary.r, c.primary.g, c.primary.b, 0.35),
    }
}

/// A pill-shaped status tag.
pub fn status_pill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(
            color.r, color.g, color.b, 0.16,
        ))),
        text_color: Some(color),
        border: Border {
            color,
            width: 1.0,
            radius: Radius::from(999.0),
        },
        ..Default::default()
    }
}
