//! Meshtastic shared-contact links.
//!
//! A contact is a [`SharedContact`] protobuf: the node number plus the node's
//! [`User`](meshtastic_protobufs::meshtastic::User), which carries the public
//! key used for PKC (end-to-end encrypted) direct messages. The official
//! clients share a contact by protobuf-encoding it, base64url-encoding the
//! bytes and embedding them in a link:
//!
//! ```text
//! https://meshtastic.org/v/#<base64url>
//! ```
//!
//! The link can be shown as a QR code or pasted as text. Importing one sends
//! an `AdminMessage.AddContact` to the radio, which stores the public key and
//! then propagates it to the mesh through its next NodeInfo broadcast.
//!
//! This module is pure: it only encodes and decodes the payload, and never
//! talks to a device.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use meshtastic_protobufs::meshtastic::{SharedContact, User};
use prost::Message;

/// Host and path prefix the official clients use for contact links.
pub const CONTACT_URL_PREFIX: &str = "https://meshtastic.org/v/#";

/// Errors that can occur when decoding a shared-contact link.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContactError {
    /// The input is empty.
    #[error("enter a contact link")]
    Empty,
    /// The input looks like a URL but is not a Meshtastic contact link.
    #[error("that is not a Meshtastic contact link")]
    NotAContactLink,
    /// The payload is not valid base64.
    #[error("the contact payload is not valid base64")]
    Base64,
    /// The payload decoded, but is not a valid `SharedContact`.
    #[error("the payload is not a Meshtastic contact")]
    Decode,
}

/// Build a [`SharedContact`] for a node from its local user record.
///
/// Returns `None` when the user has no public key, because firmware treats an
/// empty key as "clear the stored key" rather than "no change".
pub fn shared_contact_for(node_num: u32, user: &User) -> Option<SharedContact> {
    if !has_public_key(user) {
        return None;
    }
    Some(SharedContact {
        node_num,
        user: Some(user.clone()),
        should_ignore: false,
    })
}

/// Whether a user carries a usable (present and non-zero) public key.
pub fn has_public_key(user: &User) -> bool {
    !user.public_key.is_empty() && user.public_key.iter().any(|byte| *byte != 0)
}

/// Encode a shared contact as its Meshtastic link.
pub fn shared_contact_url(contact: &SharedContact) -> String {
    let bytes = contact.encode_to_vec();
    format!("{CONTACT_URL_PREFIX}{}", URL_SAFE.encode(bytes))
}

/// Parse a Meshtastic shared-contact link or a bare base64 payload.
///
/// The `meshtastic.org/v/#<payload>` form is accepted with or without a
/// leading `www.`, and with extra path segments (as the official clients
/// allow). A bare payload is accepted too, so a link whose prefix has been
/// stripped still imports.
pub fn parse_shared_contact(input: &str) -> Result<SharedContact, ContactError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ContactError::Empty);
    }

    let payload = if input.starts_with("http://") || input.starts_with("https://") {
        if !is_contact_url(input) {
            return Err(ContactError::NotAContactLink);
        }
        let fragment = input
            .split_once('#')
            .map(|(_, fragment)| fragment)
            .ok_or(ContactError::Empty)?;
        // Ignore any query string the link may carry.
        fragment.split_once('?').map(|(f, _)| f).unwrap_or(fragment)
    } else {
        input
    };

    decode_payload(payload)
}

/// Whether a URL is a Meshtastic contact link (`meshtastic.org`, path `v`).
fn is_contact_url(input: &str) -> bool {
    let Some((_, rest)) = input.split_once("://") else {
        return false;
    };
    let Some((authority, path_and_fragment)) = rest.split_once('/') else {
        return false;
    };
    let host = authority.to_ascii_lowercase();
    if host != "meshtastic.org" && host != "www.meshtastic.org" {
        return false;
    }
    let path = path_and_fragment
        .split(['#', '?'])
        .next()
        .unwrap_or_default();
    path.split('/')
        .any(|segment| segment.eq_ignore_ascii_case("v"))
}

/// Decode a base64 payload in either alphabet, restoring any missing padding.
fn decode_payload(payload: &str) -> Result<SharedContact, ContactError> {
    if payload.is_empty() {
        return Err(ContactError::Empty);
    }
    // Accept the URL-safe and standard alphabets, as the official clients do.
    let mut normalized = payload.replace('-', "+").replace('_', "/");
    while normalized.len() % 4 != 0 {
        normalized.push('=');
    }
    let bytes = STANDARD
        .decode(normalized.as_bytes())
        .map_err(|_| ContactError::Base64)?;
    SharedContact::decode(bytes.as_slice()).map_err(|_| ContactError::Decode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str) -> User {
        User {
            id: "!000000a1".into(),
            long_name: name.into(),
            short_name: "BC".into(),
            public_key: vec![0xAB; 32],
            ..Default::default()
        }
    }

    fn contact() -> SharedContact {
        SharedContact {
            node_num: 0x0000_00A1,
            user: Some(user("Base Camp")),
            should_ignore: false,
        }
    }

    #[test]
    fn round_trips_through_the_link() {
        let original = contact();
        let url = shared_contact_url(&original);
        assert!(url.starts_with(CONTACT_URL_PREFIX));
        let parsed = parse_shared_contact(&url).expect("valid link");
        assert_eq!(parsed, original);
    }

    #[test]
    fn accepts_www_and_extra_path_segments() {
        let url = shared_contact_url(&contact())
            .replace("meshtastic.org", "www.meshtastic.org")
            .replace("/v/", "/contact/v/");
        assert!(parse_shared_contact(&url).is_ok());
    }

    #[test]
    fn accepts_a_bare_payload() {
        let url = shared_contact_url(&contact());
        let payload = url.strip_prefix(CONTACT_URL_PREFIX).unwrap();
        assert_eq!(parse_shared_contact(payload).unwrap(), contact());
    }

    #[test]
    fn ignores_a_query_string() {
        let url = format!("{}?v=1", shared_contact_url(&contact()));
        assert!(parse_shared_contact(&url).is_ok());
    }

    #[test]
    fn rejects_foreign_hosts_and_paths() {
        let url = shared_contact_url(&contact());
        assert_eq!(
            parse_shared_contact(&url.replace("meshtastic.org", "example.com")),
            Err(ContactError::NotAContactLink)
        );
        assert_eq!(
            parse_shared_contact(&url.replace("/v/", "/wrong/")),
            Err(ContactError::NotAContactLink)
        );
    }

    #[test]
    fn rejects_empty_and_garbage() {
        assert_eq!(parse_shared_contact("  "), Err(ContactError::Empty));
        assert_eq!(
            parse_shared_contact("not base64!!!"),
            Err(ContactError::Base64)
        );
    }

    #[test]
    fn a_keyless_user_is_not_shareable() {
        let mut keyless = user("No Key");
        keyless.public_key.clear();
        assert!(!has_public_key(&keyless));
        assert!(shared_contact_for(1, &keyless).is_none());

        let mut zeroed = user("Zero Key");
        zeroed.public_key = vec![0; 32];
        assert!(!has_public_key(&zeroed));
        assert!(shared_contact_for(1, &zeroed).is_none());
    }
}
