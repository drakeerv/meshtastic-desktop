//! Host integration: read the machine's timezone and clock.
//!
//! The device stores its timezone as a POSIX `TZ` definition
//! (`DeviceConfig.tzdef`) rather than an IANA name, so "fill from host" has to
//! convert. Rather than shipping a timezone database, we read the definition
//! straight out of the system's compiled zoneinfo file: a TZif file ends with
//! `\n<POSIX TZ>\n`, which is exactly the string the firmware wants.

use std::path::Path;

/// Where the system keeps its compiled timezone files.
const ZONEINFO: &str = "/usr/share/zoneinfo";

/// The host's IANA timezone name, if one can be determined.
pub fn timezone_name() -> Option<String> {
    // An explicit TZ wins, but only when it names a zone; a raw POSIX string
    // in TZ is already what we want, and `posix_tzdef` handles that directly.
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim().trim_start_matches(':');
        if tz.contains('/') {
            return Some(tz.to_string());
        }
    }

    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        if let Some(name) = zoneinfo_name(&target) {
            return Some(name);
        }
    }

    // Debian keeps the name here even when /etc/localtime is a plain copy.
    if let Ok(text) = std::fs::read_to_string("/etc/timezone") {
        let name = text.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }

    None
}

/// The POSIX `TZ` definition matching the host timezone.
pub fn posix_tzdef() -> Option<String> {
    // A raw POSIX TZ in the environment can be sent to the device as-is.
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim().trim_start_matches(':');
        if !tz.is_empty() && !tz.contains('/') {
            return Some(tz.to_string());
        }
    }

    let name = timezone_name()?;
    let bytes = std::fs::read(format!("{ZONEINFO}/{name}")).ok()?;
    tzif_footer(&bytes)
}

/// The IANA name embedded in a zoneinfo path such as
/// `/usr/share/zoneinfo/America/New_York`.
fn zoneinfo_name(path: &Path) -> Option<String> {
    path.to_string_lossy()
        .split("zoneinfo/")
        .nth(1)
        .map(|name| name.trim_start_matches('/').to_string())
        .filter(|name| !name.is_empty() && !name.contains(".."))
}

/// Extract the POSIX `TZ` string from a TZif file's trailing footer.
fn tzif_footer(bytes: &[u8]) -> Option<String> {
    // The footer is `\n<TZ string>\n`, so the string sits between the last two
    // newlines.
    let body = bytes.strip_suffix(b"\n")?;
    let start = body.iter().rposition(|byte| *byte == b'\n')? + 1;
    let text = std::str::from_utf8(&body[start..]).ok()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn reads_the_tzif_footer() {
        let data = b"TZif2\x00binary\x00data\nPST8PDT,M3.2.0,M11.1.0\n";
        assert_eq!(tzif_footer(data).as_deref(), Some("PST8PDT,M3.2.0,M11.1.0"));
    }

    #[test]
    fn rejects_a_missing_or_empty_footer() {
        assert_eq!(tzif_footer(b"not a tzif"), None);
        assert_eq!(tzif_footer(b"TZif2\x00\n\n"), None);
    }

    #[test]
    fn extracts_the_zone_name_from_a_path() {
        let path = PathBuf::from("/usr/share/zoneinfo/America/New_York");
        assert_eq!(zoneinfo_name(&path).as_deref(), Some("America/New_York"));
        assert_eq!(zoneinfo_name(&PathBuf::from("/etc/localtime")), None);
    }
}
