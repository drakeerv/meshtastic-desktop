//! Small presentation helpers: node names, signal/battery labels, relative
//! times and distances.

use meshtastic_protobufs::meshtastic::{NodeInfo, PortNum};
use mt_persistence::MessageStatus;

/// `!a1b2c3d4` style identifier for a node number.
pub fn node_id(num: u32) -> String {
    format!("!{num:08x}")
}

/// The best available display name for a node.
pub fn node_name(node: &NodeInfo) -> String {
    if let Some(user) = &node.user {
        if !user.long_name.trim().is_empty() {
            return user.long_name.clone();
        }
        if !user.short_name.trim().is_empty() {
            return user.short_name.clone();
        }
    }
    node_id(node.num)
}

/// The short name, falling back to the id suffix.
pub fn node_short_name(node: &NodeInfo) -> String {
    if let Some(user) = &node.user {
        if !user.short_name.trim().is_empty() {
            return user.short_name.clone();
        }
    }
    node_id(node.num)
}

/// A node's hardware model as a friendly label.
pub fn hw_model_label(model: i32) -> &'static str {
    // Models newer than the pinned protobufs crate (raw enum ids).
    match model {
        95 => return "Seeed Solar Node",
        96 => return "Nomadstar Meteor Pro",
        97 => return "CrowPanel",
        99 => return "Seeed Wio Tracker L1",
        100 => return "Seeed Wio Tracker L1 E-Ink",
        101 => return "Muzi R1 Neo",
        102 => return "T-Deck Pro",
        103 => return "T-Lora Pager",
        105 => return "RAK WisMesh Tag",
        106 => return "RAK3312",
        108 => return "Heltec Mesh Solar",
        109 => return "T-Echo Lite",
        110 => return "Heltec V4",
        111 => return "M5Stack C6L",
        113 => return "Heltec Tracker V2",
        116 => return "RAK WisMesh Tap V2",
        122 => return "T-Beam 1 Watt",
        128 => return "Tracker T1000-E Pro",
        132 => return "Heltec V4 R8",
        133 => return "Heltec Mesh Node T1",
        134 => return "Station G3",
        136 => return "T-Echo Card",
        139 => return "Heltec Mesh Tower V2",
        _ => {}
    }
    use meshtastic_protobufs::meshtastic::HardwareModel as H;
    match H::try_from(model) {
        Ok(H::HeltecV3) => "Heltec V3",
        Ok(H::HeltecV21) => "Heltec V2.1",
        Ok(H::HeltecV20) => "Heltec V2.0",
        Ok(H::HeltecV1) => "Heltec V1",
        Ok(H::Tbeam) => "T-Beam",
        Ok(H::TbeamV0p7) => "T-Beam v0.7",
        Ok(H::TDeck) => "T-Deck",
        Ok(H::TWatchS3) => "T-Watch S3",
        Ok(H::TloraV2) => "T-Lora V2",
        Ok(H::TloraV1) => "T-Lora V1",
        Ok(H::TloraT3S3) => "T-Lora T3-S3",
        Ok(H::Rak4631) => "RAK4631",
        Ok(H::Rak11200) => "RAK11200",
        Ok(H::Rak11310) => "RAK11310",
        Ok(H::StationG1) => "Station G1",
        Ok(H::StationG2) => "Station G2",
        Ok(H::NanoG1) => "Nano G1",
        Ok(H::NanoG2Ultra) => "Nano G2 Ultra",
        Ok(H::M5stack) => "M5Stack",
        Ok(H::RpiPico) => "Raspberry Pi Pico",
        Ok(H::Nrf52840dk) => "nRF52840 DK",
        Ok(H::Portduino) => "Portduino",
        Ok(H::AndroidSim) => "Android Simulator",
        Ok(H::DiyV1) => "DIY v1",
        Ok(H::Unset) => "Unknown",
        _ => "Device",
    }
}

/// Relative "last heard" text.
pub fn relative_time(timestamp: u32, now: i64) -> String {
    if timestamp == 0 {
        return "never".to_string();
    }
    let delta = now - timestamp as i64;
    if delta < 0 {
        // Future timestamps usually mean an unsynced node clock.
        return "just now".to_string();
    }
    match delta {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{}m ago", delta / 60),
        3_600..=86_399 => format!("{}h ago", delta / 3_600),
        _ => format!("{}d ago", delta / 86_400),
    }
}

/// Compact "last heard" for dense lists.
pub fn last_heard_short(timestamp: u32, now: i64) -> String {
    if timestamp == 0 {
        return "-".to_string();
    }
    let delta = now - timestamp as i64;
    if delta <= 0 {
        return "now".to_string();
    }
    match delta {
        0..=59 => "now".to_string(),
        60..=3_599 => format!("{}m", delta / 60),
        3_600..=86_399 => format!("{}h", delta / 3_600),
        _ => format!("{}d", delta / 86_400),
    }
}

/// Signal quality label from SNR.
pub fn snr_label(snr: f32) -> &'static str {
    match snr {
        s if s >= 5.0 => "excellent",
        s if s >= 0.0 => "good",
        s if s >= -7.0 => "fair",
        s if s >= -15.0 => "weak",
        _ => "very weak",
    }
}

/// Approximate signal bars (0-4) from SNR.
pub fn snr_bars(snr: f32) -> u8 {
    match snr {
        s if s >= 5.0 => 4,
        s if s >= 0.0 => 3,
        s if s >= -7.0 => 2,
        s if s >= -15.0 => 1,
        _ => 0,
    }
}

/// Battery level label. Values above 100% mean the node is on external
/// power (the firmware reports 101).
pub fn battery_label(level: Option<u32>) -> String {
    match level {
        Some(level) if level > 100 => "external".to_string(),
        Some(level) => format!("{level}%"),
        None => "-".to_string(),
    }
}

/// "3 hops" / "direct" / "-".
pub fn hops_label(hops: Option<u32>) -> String {
    match hops {
        Some(0) => "direct".to_string(),
        Some(1) => "1 hop".to_string(),
        Some(n) => format!("{n} hops"),
        None => "-".to_string(),
    }
}

/// Great-circle distance in kilometres between two `1e-7` degree positions.
pub fn distance_km(a: (i32, i32), b: (i32, i32)) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6_371.0;
    let lat1 = a.0 as f64 * 1e-7_f64.to_radians();
    let lon1 = a.1 as f64 * 1e-7_f64.to_radians();
    let lat2 = b.0 as f64 * 1e-7_f64.to_radians();
    let lon2 = b.1 as f64 * 1e-7_f64.to_radians();
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * h.sqrt().asin()
}

/// Format a distance according to the unit preference.
pub fn format_distance(km: f64, imperial: bool) -> String {
    if imperial {
        let miles = km * 0.621_371;
        if miles < 0.1 {
            format!("{:.0} ft", miles * 5_280.0)
        } else {
            format!("{miles:.1} mi")
        }
    } else if km < 0.1 {
        format!("{:.0} m", km * 1_000.0)
    } else {
        format!("{km:.1} km")
    }
}

/// Local wall-clock time (`HH:MM`) for a Unix timestamp.
pub fn clock_time(unix: i64) -> String {
    use jiff::tz::TimeZone;
    match jiff::Timestamp::from_second(unix) {
        Ok(timestamp) => {
            let zoned = timestamp.to_zoned(TimeZone::system());
            format!("{:02}:{:02}", zoned.hour(), zoned.minute())
        }
        Err(_) => String::new(),
    }
}

/// A day-separator label: "Today", "Yesterday", or a date such as "Sep 18".
/// Older years include the year.
pub fn day_label(unix: i64, now: i64) -> String {
    let Some((year, month, day)) = date_parts(unix) else {
        return String::new();
    };
    if Some((year, month, day)) == date_parts(now) {
        return "Today".to_string();
    }
    if Some((year, month, day)) == date_parts(now - 86_400) {
        return "Yesterday".to_string();
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let name = MONTHS
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("");
    let current_year = date_parts(now).map(|parts| parts.0).unwrap_or(year);
    if year == current_year {
        format!("{name} {day}")
    } else {
        format!("{name} {day}, {year}")
    }
}

/// The calendar date `(year, month, day)` of a Unix timestamp in local time.
fn date_parts(unix: i64) -> Option<(i16, i8, i8)> {
    use jiff::tz::TimeZone;
    let timestamp = jiff::Timestamp::from_second(unix).ok()?;
    let zoned = timestamp.to_zoned(TimeZone::system());
    Some((zoned.year(), zoned.month(), zoned.day()))
}

/// Human label for a message's port number.
pub fn portnum_label(portnum: i32) -> &'static str {
    match PortNum::try_from(portnum) {
        Ok(PortNum::TextMessageApp) => "text",
        Ok(PortNum::PositionApp) => "position",
        Ok(PortNum::NodeinfoApp) => "node info",
        Ok(PortNum::TelemetryApp) => "telemetry",
        Ok(PortNum::RoutingApp) => "routing",
        Ok(PortNum::AdminApp) => "admin",
        Ok(PortNum::TracerouteApp) => "traceroute",
        Ok(PortNum::AlertApp) => "alert",
        Ok(PortNum::StoreForwardApp) => "store & forward",
        _ => "packet",
    }
}

pub fn status_label(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Queued => "queued",
        MessageStatus::Enroute => "sending",
        MessageStatus::Delivered => "delivered",
        MessageStatus::Failed => "failed",
    }
}

/// A channel's display name, falling back to `Channel N`.
pub fn channel_name(name: Option<&str>, index: u32) -> String {
    match name {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ if index == 0 => "Primary".to_string(),
        _ => format!("Channel {index}"),
    }
}

/// A LoRa modem preset's display name, e.g. `LongFast`.
pub fn modem_preset_label(preset: i32) -> String {
    use meshtastic_protobufs::meshtastic::config::lo_ra_config::ModemPreset;
    match ModemPreset::try_from(preset) {
        Ok(ModemPreset::LongFast) => "LongFast".to_string(),
        Ok(ModemPreset::LongSlow) => "LongSlow".to_string(),
        Ok(ModemPreset::VeryLongSlow) => "VeryLongSlow".to_string(),
        Ok(ModemPreset::MediumSlow) => "MediumSlow".to_string(),
        Ok(ModemPreset::MediumFast) => "MediumFast".to_string(),
        Ok(ModemPreset::ShortSlow) => "ShortSlow".to_string(),
        Ok(ModemPreset::ShortFast) => "ShortFast".to_string(),
        Ok(ModemPreset::LongModerate) => "LongModerate".to_string(),
        Ok(ModemPreset::ShortTurbo) => "ShortTurbo".to_string(),
        Err(_) => format!("Preset {preset}"),
    }
}

/// Clean a device log line for display: strip ANSI escape sequences (the
/// firmware emits coloured console output) and other control characters.
pub fn sanitize_log_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            // Escape sequences: CSI (`ESC [ ... final`) and OSC (`ESC ] ...`).
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            // Tabs become spaces; other control characters are dropped.
            '\t' => out.push(' '),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }

    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_colour_codes() {
        let raw = "\u{1b}[0m\u{1b}[34mDEBUG \u{1b}[0m| 02:11:27 [GPS] trying";
        assert_eq!(sanitize_log_line(raw), "DEBUG | 02:11:27 [GPS] trying");
    }

    #[test]
    fn strips_osc_sequences() {
        let raw = "before\u{1b}]0;title\u{7}after";
        assert_eq!(sanitize_log_line(raw), "beforeafter");
    }

    #[test]
    fn drops_control_characters_but_keeps_text() {
        assert_eq!(sanitize_log_line("a\tb\r\nc"), "a bc");
    }

    #[test]
    fn plain_lines_pass_through() {
        let raw = "[   604][I][esp32-hal-psram.c:96] psramInit(): PSRAM enabled";
        assert_eq!(sanitize_log_line(raw), raw);
        assert_eq!(sanitize_log_line(""), "");
    }

    #[test]
    fn modem_preset_names() {
        assert_eq!(modem_preset_label(0), "LongFast");
        assert_eq!(modem_preset_label(8), "ShortTurbo");
        assert_eq!(modem_preset_label(99), "Preset 99");
    }
}

/// Turn bare URLs in a message into Markdown links so they can be clicked.
///
/// The Markdown renderer does not enable GFM autolinks, so we wrap plain
/// `http(s)://` and `www.` URLs ourselves. URLs already inside an explicit
/// link or autolink are left alone.
pub fn autolink(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;

    while index < text.len() {
        if let Some(length) = url_at(&text[index..]) {
            let boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
            let inside_link = matches!(
                bytes.get(index.wrapping_sub(1)),
                Some(b'(' | b'<' | b'"' | b'\'')
            );
            if boundary && !inside_link {
                let url = &text[index..index + length];
                let target = if url.starts_with("www.") {
                    format!("https://{url}")
                } else {
                    url.to_string()
                };
                out.push('[');
                out.push_str(url);
                out.push_str("](");
                out.push_str(&target);
                out.push(')');
                index += length;
                continue;
            }
        }

        let ch = text[index..].chars().next().unwrap();
        out.push(ch);
        index += ch.len_utf8();
    }

    out
}

/// Length of a bare URL at the start of `text`, if any (trailing punctuation
/// is excluded).
fn url_at(text: &str) -> Option<usize> {
    if !(text.starts_with("http://") || text.starts_with("https://") || text.starts_with("www.")) {
        return None;
    }

    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            break;
        }
        end = idx + ch.len_utf8();
    }

    while end > 0 {
        let ch = text[..end].chars().last().unwrap();
        if matches!(
            ch,
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\''
        ) {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }

    (end > 0).then_some(end)
}

#[cfg(test)]
mod autolink_tests {
    use super::*;

    #[test]
    fn autolinks_bare_urls() {
        assert_eq!(
            autolink("see https://example.com/page now"),
            "see [https://example.com/page](https://example.com/page) now"
        );
    }

    #[test]
    fn autolink_trims_trailing_punctuation() {
        assert_eq!(
            autolink("go to https://example.com."),
            "go to [https://example.com](https://example.com)."
        );
    }

    #[test]
    fn autolink_leaves_existing_links_alone() {
        let input = "[docs](https://example.com)";
        assert_eq!(autolink(input), input);
        let auto = "<https://example.com>";
        assert_eq!(autolink(auto), auto);
    }

    #[test]
    fn autolink_adds_scheme_for_www() {
        assert_eq!(
            autolink("www.example.com"),
            "[www.example.com](https://www.example.com)"
        );
    }

    #[test]
    fn autolink_ignores_plain_text() {
        assert_eq!(autolink("hello world"), "hello world");
    }
}

/// A device role's display name (config `DeviceConfig.Role`).
pub fn role_label(role: i32) -> &'static str {
    use meshtastic_protobufs::meshtastic::config::device_config::Role;
    match Role::try_from(role) {
        Ok(Role::Client) => "Client",
        Ok(Role::ClientMute) => "Client Mute",
        Ok(Role::Router) => "Router",
        Ok(Role::RouterClient) => "Router Client",
        Ok(Role::Repeater) => "Repeater",
        Ok(Role::Tracker) => "Tracker",
        Ok(Role::Sensor) => "Sensor",
        Ok(Role::Tak) => "TAK",
        Ok(Role::ClientHidden) => "Client Hidden",
        Ok(Role::LostAndFound) => "Lost and Found",
        Ok(Role::TakTracker) => "TAK Tracker",
        Ok(Role::RouterLate) => "Router Late",
        Err(_) => "Unknown",
    }
}

/// How a node was last heard. The pinned protobufs do not carry the transport
/// mechanism on `NodeInfo`, so MQTT is distinguished and the rest is LoRa.
pub fn transport_label(via_mqtt: bool) -> &'static str {
    if via_mqtt { "MQTT" } else { "LoRa" }
}

/// A node's public key as base64, or `None` when it has not shared one.
pub fn public_key_label(key: &[u8]) -> Option<String> {
    use base64::Engine as _;
    if key.is_empty() {
        return None;
    }
    Some(base64::engine::general_purpose::STANDARD.encode(key))
}

#[cfg(test)]
mod label_tests {
    use super::*;

    #[test]
    fn role_labels_are_human_readable() {
        assert_eq!(role_label(0), "Client");
        assert_eq!(role_label(2), "Router");
        assert_eq!(role_label(8), "Client Hidden");
        assert_eq!(role_label(11), "Router Late");
        assert_eq!(role_label(99), "Unknown");
    }

    #[test]
    fn public_key_is_base64_or_none() {
        assert_eq!(public_key_label(&[]), None);
        assert_eq!(public_key_label(&[0xAB; 32]).unwrap().len(), 44);
    }

    #[test]
    fn transport_is_mqtt_or_lora() {
        assert_eq!(transport_label(true), "MQTT");
        assert_eq!(transport_label(false), "LoRa");
    }
}
