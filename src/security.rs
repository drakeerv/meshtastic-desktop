//! Security indicators for direct messages and channels.
//!
//! A direct message is encrypted with the destination node's public key
//! (PKC), so it is end-to-end encrypted once that key is on file. A channel
//! uses a shared pre-shared key (PSK); the key's length sets the cipher
//! strength. These helpers turn that into a short label, an icon and a colour
//! so every surface (message header, sidebar, node detail) reads the same.

use iced::widget::container;
use iced::{Alignment, Border, Color, Element};

use meshtastic_protobufs::meshtastic::User;

use crate::app::Message;
use crate::icons::lucide;
use crate::theme;

/// How well a node's direct messages are protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSecurity {
    /// The user verified this node in person, or it is the local radio.
    Verified,
    /// A usable public key is on file; direct messages are encrypted.
    Encrypted,
    /// No public key on file; direct messages are not end-to-end encrypted.
    NoKey,
    /// A different key arrived than the one on file.
    Mismatch,
}

impl NodeSecurity {
    /// Classify a node from its user record and verification flags.
    pub fn of(user: Option<&User>, manually_verified: bool, is_local: bool) -> Self {
        let key = user.map(|u| u.public_key.as_slice()).unwrap_or(&[]);
        let usable = !key.is_empty() && key.iter().any(|byte| *byte != 0);
        if !key.is_empty() && !usable {
            Self::Mismatch
        } else if manually_verified || is_local {
            Self::Verified
        } else if usable {
            Self::Encrypted
        } else {
            Self::NoKey
        }
    }

    /// Short label for a chip or row.
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "Verified",
            Self::Encrypted => "Encrypted",
            Self::NoKey => "Not encrypted",
            Self::Mismatch => "Key mismatch",
        }
    }

    /// One-sentence explanation for the node detail panel.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Verified => {
                "You verified this node's key in person; direct messages are end-to-end encrypted."
            }
            Self::Encrypted => {
                "A public key is on file; direct messages to this node are end-to-end encrypted."
            }
            Self::NoKey => {
                "No public key on file; direct messages to this node are not end-to-end encrypted."
            }
            Self::Mismatch => {
                "A different public key arrived than the one on file; messages may not reach the node you expect."
            }
        }
    }

    /// Severity colour.
    pub fn color(self) -> Color {
        match self {
            Self::Verified | Self::Encrypted => theme::primary(),
            Self::NoKey => theme::warning(),
            Self::Mismatch => theme::danger(),
        }
    }

    /// The icon at `size`.
    pub fn icon(self, size: f32) -> Element<'static, Message> {
        let color = self.color();
        match self {
            Self::Verified => lucide::shield_check().size(size).color(color).into(),
            Self::Encrypted => lucide::lock().size(size).color(color).into(),
            Self::NoKey => lucide::lock_open().size(size).color(color).into(),
            Self::Mismatch => lucide::key_round().size(size).color(color).into(),
        }
    }

    /// A compact pill for headers and list rows.
    pub fn chip(self) -> Element<'static, Message> {
        pill(self.icon(11.0), self.label(), self.color())
    }
}

/// How strongly a channel's pre-shared key protects its traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSecurity {
    /// No key, or the well-known one-byte default key.
    None,
    /// A one-byte key: effectively the public default.
    DefaultKey,
    /// A 128-bit AES key.
    Aes128,
    /// A 256-bit AES key.
    Aes256,
}

impl ChannelSecurity {
    /// Classify a channel from its raw PSK bytes.
    pub fn of(psk: &[u8]) -> Self {
        match psk.len() {
            0 => Self::None,
            1 => Self::DefaultKey,
            16 => Self::Aes128,
            32 => Self::Aes256,
            _ => Self::Aes256,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No encryption",
            Self::DefaultKey => "Default key",
            Self::Aes128 => "AES-128",
            Self::Aes256 => "AES-256",
        }
    }

    fn secure(self) -> bool {
        matches!(self, Self::Aes128 | Self::Aes256)
    }

    pub fn color(self) -> Color {
        if self.secure() {
            theme::primary()
        } else {
            theme::warning()
        }
    }

    pub fn icon(self, size: f32) -> Element<'static, Message> {
        let color = self.color();
        if self.secure() {
            lucide::lock().size(size).color(color).into()
        } else {
            lucide::lock_open().size(size).color(color).into()
        }
    }

    pub fn chip(self) -> Element<'static, Message> {
        pill(self.icon(11.0), self.label(), self.color())
    }
}

/// A small rounded label with an icon and a tinted background.
fn pill(
    icon: Element<'static, Message>,
    label: &'static str,
    color: Color,
) -> Element<'static, Message> {
    container(
        iced::widget::row![icon, iced::widget::text(label).size(11).color(color),]
            .spacing(5)
            .align_y(Alignment::Center),
    )
    .padding([3, 8])
    .style(move |_| container::Style {
        background: Some(Color::from_rgba(color.r, color.g, color.b, 0.12).into()),
        border: Border {
            color,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::User;

    fn user(key: Vec<u8>) -> User {
        User {
            public_key: key,
            ..Default::default()
        }
    }

    #[test]
    fn node_security_classifies_keys() {
        assert_eq!(
            NodeSecurity::of(Some(&user(vec![0xAB; 32])), false, false),
            NodeSecurity::Encrypted
        );
        // A verified contact and the local radio both read as Verified.
        assert_eq!(
            NodeSecurity::of(Some(&user(vec![0xAB; 32])), true, false),
            NodeSecurity::Verified
        );
        assert_eq!(
            NodeSecurity::of(Some(&user(vec![0xAB; 32])), false, true),
            NodeSecurity::Verified
        );
        // An all-zero key is the mismatch sentinel, not "no key".
        assert_eq!(
            NodeSecurity::of(Some(&user(vec![0; 32])), false, false),
            NodeSecurity::Mismatch
        );
        assert_eq!(
            NodeSecurity::of(Some(&user(Vec::new())), false, false),
            NodeSecurity::NoKey
        );
        assert_eq!(NodeSecurity::of(None, false, false), NodeSecurity::NoKey);
    }

    #[test]
    fn channel_security_classifies_key_lengths() {
        assert_eq!(ChannelSecurity::of(&[]), ChannelSecurity::None);
        assert_eq!(ChannelSecurity::of(&[0x01]), ChannelSecurity::DefaultKey);
        assert_eq!(ChannelSecurity::of(&[0; 16]), ChannelSecurity::Aes128);
        assert_eq!(ChannelSecurity::of(&[0; 32]), ChannelSecurity::Aes256);
        // An unexpected length is treated as strong rather than alarming.
        assert_eq!(ChannelSecurity::of(&[0; 8]), ChannelSecurity::Aes256);
    }
}
