//! Vendored Meshtastic artwork: the in-app logo mark, the app icon, and one
//! board illustration per hardware model.
//!
//! Everything lives under `assets/meshtastic/` (see the attribution file
//! there), is embedded in the binary, and is never read from disk or from
//! `references/` at runtime.
//!
//! [`device_file`] maps a `HardwareModel` **numeric id** to a file. It works
//! on raw ids rather than the generated enum so that models newer than the
//! pinned `meshtastic_protobufs` crate (Heltec V4, Station G3, ...) resolve.

use iced::widget::svg::Handle;

/// Every bundled device illustration.
static FILES: &[(&str, &[u8])] = &[
    (
        "crowpanel_2_4.svg",
        include_bytes!("../assets/meshtastic/devices/crowpanel_2_4.svg"),
    ),
    (
        "crowpanel_2_8.svg",
        include_bytes!("../assets/meshtastic/devices/crowpanel_2_8.svg"),
    ),
    (
        "crowpanel_3_5.svg",
        include_bytes!("../assets/meshtastic/devices/crowpanel_3_5.svg"),
    ),
    (
        "crowpanel_5_0.svg",
        include_bytes!("../assets/meshtastic/devices/crowpanel_5_0.svg"),
    ),
    (
        "crowpanel_7_0.svg",
        include_bytes!("../assets/meshtastic/devices/crowpanel_7_0.svg"),
    ),
    (
        "diy.svg",
        include_bytes!("../assets/meshtastic/devices/diy.svg"),
    ),
    (
        "heltec-ht62-esp32c3-sx1262.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-ht62-esp32c3-sx1262.svg"),
    ),
    (
        "heltec-mesh-node-t114-case.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-mesh-node-t114-case.svg"),
    ),
    (
        "heltec-mesh-node-t114.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-mesh-node-t114.svg"),
    ),
    (
        "heltec-mesh-solar.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-mesh-solar.svg"),
    ),
    (
        "heltec-v3-case.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-v3-case.svg"),
    ),
    (
        "heltec-v3.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-v3.svg"),
    ),
    (
        "heltec-vision-master-e213.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-vision-master-e213.svg"),
    ),
    (
        "heltec-vision-master-e290.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-vision-master-e290.svg"),
    ),
    (
        "heltec-vision-master-t190.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-vision-master-t190.svg"),
    ),
    (
        "heltec-wireless-paper-V1_0.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-wireless-paper-V1_0.svg"),
    ),
    (
        "heltec-wireless-paper.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-wireless-paper.svg"),
    ),
    (
        "heltec-wireless-tracker-V1-0.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-wireless-tracker-V1-0.svg"),
    ),
    (
        "heltec-wireless-tracker.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-wireless-tracker.svg"),
    ),
    (
        "heltec-wsl-v3.svg",
        include_bytes!("../assets/meshtastic/devices/heltec-wsl-v3.svg"),
    ),
    (
        "heltec_mesh_pocket.svg",
        include_bytes!("../assets/meshtastic/devices/heltec_mesh_pocket.svg"),
    ),
    (
        "heltec_v4.svg",
        include_bytes!("../assets/meshtastic/devices/heltec_v4.svg"),
    ),
    (
        "lilygo-tlora-pager.svg",
        include_bytes!("../assets/meshtastic/devices/lilygo-tlora-pager.svg"),
    ),
    (
        "m5_c6l.svg",
        include_bytes!("../assets/meshtastic/devices/m5_c6l.svg"),
    ),
    (
        "meteor_pro.svg",
        include_bytes!("../assets/meshtastic/devices/meteor_pro.svg"),
    ),
    (
        "muzi_r1_neo.svg",
        include_bytes!("../assets/meshtastic/devices/muzi_r1_neo.svg"),
    ),
    (
        "nano-g2-ultra.svg",
        include_bytes!("../assets/meshtastic/devices/nano-g2-ultra.svg"),
    ),
    (
        "pico.svg",
        include_bytes!("../assets/meshtastic/devices/pico.svg"),
    ),
    (
        "promicro.svg",
        include_bytes!("../assets/meshtastic/devices/promicro.svg"),
    ),
    (
        "rak-wismesh-tap-v2.svg",
        include_bytes!("../assets/meshtastic/devices/rak-wismesh-tap-v2.svg"),
    ),
    (
        "rak-wismeshtap.svg",
        include_bytes!("../assets/meshtastic/devices/rak-wismeshtap.svg"),
    ),
    (
        "rak11200.svg",
        include_bytes!("../assets/meshtastic/devices/rak11200.svg"),
    ),
    (
        "rak11310.svg",
        include_bytes!("../assets/meshtastic/devices/rak11310.svg"),
    ),
    (
        "rak2560.svg",
        include_bytes!("../assets/meshtastic/devices/rak2560.svg"),
    ),
    (
        "rak4631.svg",
        include_bytes!("../assets/meshtastic/devices/rak4631.svg"),
    ),
    (
        "rak4631_case.svg",
        include_bytes!("../assets/meshtastic/devices/rak4631_case.svg"),
    ),
    (
        "rak_3312.svg",
        include_bytes!("../assets/meshtastic/devices/rak_3312.svg"),
    ),
    (
        "rak_wismesh_tag.svg",
        include_bytes!("../assets/meshtastic/devices/rak_wismesh_tag.svg"),
    ),
    (
        "rpipicow.svg",
        include_bytes!("../assets/meshtastic/devices/rpipicow.svg"),
    ),
    (
        "seeed-sensecap-indicator.svg",
        include_bytes!("../assets/meshtastic/devices/seeed-sensecap-indicator.svg"),
    ),
    (
        "seeed-xiao-s3.svg",
        include_bytes!("../assets/meshtastic/devices/seeed-xiao-s3.svg"),
    ),
    (
        "seeed_solar.svg",
        include_bytes!("../assets/meshtastic/devices/seeed_solar.svg"),
    ),
    (
        "seeed_xiao_nrf52_kit.svg",
        include_bytes!("../assets/meshtastic/devices/seeed_xiao_nrf52_kit.svg"),
    ),
    (
        "station-g2.svg",
        include_bytes!("../assets/meshtastic/devices/station-g2.svg"),
    ),
    (
        "t-deck.svg",
        include_bytes!("../assets/meshtastic/devices/t-deck.svg"),
    ),
    (
        "t-echo.svg",
        include_bytes!("../assets/meshtastic/devices/t-echo.svg"),
    ),
    (
        "t-watch-s3.svg",
        include_bytes!("../assets/meshtastic/devices/t-watch-s3.svg"),
    ),
    (
        "tbeam-s3-core.svg",
        include_bytes!("../assets/meshtastic/devices/tbeam-s3-core.svg"),
    ),
    (
        "tbeam.svg",
        include_bytes!("../assets/meshtastic/devices/tbeam.svg"),
    ),
    (
        "tdeck_pro.svg",
        include_bytes!("../assets/meshtastic/devices/tdeck_pro.svg"),
    ),
    (
        "techo_lite.svg",
        include_bytes!("../assets/meshtastic/devices/techo_lite.svg"),
    ),
    (
        "thinknode_m1.svg",
        include_bytes!("../assets/meshtastic/devices/thinknode_m1.svg"),
    ),
    (
        "thinknode_m2.svg",
        include_bytes!("../assets/meshtastic/devices/thinknode_m2.svg"),
    ),
    (
        "tlora-c6.svg",
        include_bytes!("../assets/meshtastic/devices/tlora-c6.svg"),
    ),
    (
        "tlora-t3s3-epaper.svg",
        include_bytes!("../assets/meshtastic/devices/tlora-t3s3-epaper.svg"),
    ),
    (
        "tlora-t3s3-v1.svg",
        include_bytes!("../assets/meshtastic/devices/tlora-t3s3-v1.svg"),
    ),
    (
        "tlora-v2-1-1_6.svg",
        include_bytes!("../assets/meshtastic/devices/tlora-v2-1-1_6.svg"),
    ),
    (
        "tlora-v2-1-1_8.svg",
        include_bytes!("../assets/meshtastic/devices/tlora-v2-1-1_8.svg"),
    ),
    (
        "tracker-t1000-e.svg",
        include_bytes!("../assets/meshtastic/devices/tracker-t1000-e.svg"),
    ),
    (
        "unknown.svg",
        include_bytes!("../assets/meshtastic/devices/unknown.svg"),
    ),
    (
        "wio-tracker-wm1110.svg",
        include_bytes!("../assets/meshtastic/devices/wio-tracker-wm1110.svg"),
    ),
    (
        "wio_tracker_l1_case.svg",
        include_bytes!("../assets/meshtastic/devices/wio_tracker_l1_case.svg"),
    ),
    (
        "wio_tracker_l1_eink.svg",
        include_bytes!("../assets/meshtastic/devices/wio_tracker_l1_eink.svg"),
    ),
    (
        "wm1110_dev_kit.svg",
        include_bytes!("../assets/meshtastic/devices/wm1110_dev_kit.svg"),
    ),
];

fn bytes_for(file: &str) -> &'static [u8] {
    FILES
        .iter()
        .find(|(name, _)| *name == file)
        .map(|(_, bytes)| *bytes)
        .unwrap_or_else(|| bytes_for("unknown.svg"))
}

/// The file name (under `assets/meshtastic/devices/`) for a hardware model id.
pub fn device_file(model: i32) -> &'static str {
    match model {
        1 => "tlora-v2-1-1_6.svg",
        2 => "tlora-v2-1-1_6.svg",
        3 => "tlora-v2-1-1_6.svg",
        4 => "tbeam.svg",
        5 => "heltec-v3.svg",
        6 => "tbeam.svg",
        7 => "t-echo.svg",
        8 => "tlora-v2-1-1_6.svg",
        9 => "rak4631.svg",
        10 => "heltec-v3.svg",
        11 => "heltec-v3.svg",
        12 => "tbeam-s3-core.svg",
        13 => "rak11200.svg",
        15 => "tlora-v2-1-1_8.svg",
        16 => "tlora-t3s3-v1.svg",
        18 => "nano-g2-ultra.svg",
        21 => "wio-tracker-wm1110.svg",
        22 => "rak2560.svg",
        25 => "station-g2.svg",
        26 => "rak11310.svg",
        31 => "station-g2.svg",
        33 => "t-echo.svg",
        39 => "diy.svg",
        43 => "heltec-v3.svg",
        44 => "heltec-wsl-v3.svg",
        47 => "pico.svg",
        48 => "heltec-wireless-tracker.svg",
        49 => "heltec-wireless-paper.svg",
        50 => "t-deck.svg",
        51 => "t-watch-s3.svg",
        53 => "heltec-ht62-esp32c3-sx1262.svg",
        57 => "heltec-wireless-paper-V1_0.svg",
        58 => "heltec-wireless-tracker-V1-0.svg",
        60 => "tlora-c6.svg",
        63 => "promicro.svg",
        66 => "heltec-vision-master-t190.svg",
        67 => "heltec-vision-master-e213.svg",
        68 => "heltec-vision-master-e290.svg",
        69 => "heltec-mesh-node-t114.svg",
        70 => "seeed-sensecap-indicator.svg",
        71 => "tracker-t1000-e.svg",
        79 => "rpipicow.svg",
        81 => "seeed-xiao-s3.svg",
        83 => "tlora-c6.svg",
        84 => "rak-wismeshtap.svg",
        88 => "seeed_xiao_nrf52_kit.svg",
        89 => "thinknode_m1.svg",
        90 => "thinknode_m2.svg",
        94 => "heltec_mesh_pocket.svg",
        95 => "seeed_solar.svg",
        96 => "meteor_pro.svg",
        97 => "crowpanel_3_5.svg",
        99 => "wio_tracker_l1_case.svg",
        100 => "wio_tracker_l1_eink.svg",
        101 => "muzi_r1_neo.svg",
        102 => "tdeck_pro.svg",
        103 => "lilygo-tlora-pager.svg",
        105 => "rak_wismesh_tag.svg",
        106 => "rak_3312.svg",
        108 => "heltec-mesh-solar.svg",
        109 => "techo_lite.svg",
        110 => "heltec_v4.svg",
        111 => "m5_c6l.svg",
        113 => "heltec-wireless-tracker.svg",
        114 => "t-watch-s3.svg",
        116 => "rak-wismesh-tap-v2.svg",
        122 => "tbeam.svg",
        124 => "tbeam.svg",
        127 => "heltec-mesh-node-t114.svg",
        128 => "tracker-t1000-e.svg",
        132 => "heltec_v4.svg",
        133 => "heltec-mesh-node-t114.svg",
        134 => "station-g2.svg",
        136 => "t-echo.svg",
        139 => "heltec-mesh-node-t114.svg",
        _ => "unknown.svg",
    }
}

/// A renderable handle for a hardware model's board illustration.
pub fn device_art(model: i32) -> Handle {
    Handle::from_memory(bytes_for(device_file(model)))
}

/// The generic placeholder board (used when no device is connected).
pub fn unknown_art() -> Handle {
    Handle::from_memory(bytes_for("unknown.svg"))
}

/// The in-app logo mark (bare mountain, no background), themed for the
/// current palette: white on dark, black on light.
pub fn logo(is_light: bool) -> Handle {
    if is_light {
        Handle::from_memory(include_bytes!("../assets/meshtastic/logo_mark_black.svg").as_slice())
    } else {
        Handle::from_memory(include_bytes!("../assets/meshtastic/logo_mark_white.svg").as_slice())
    }
}

/// The application icon (PNG bytes) for the window/taskbar and tray.
pub fn app_icon_png() -> &'static [u8] {
    include_bytes!("../assets/meshtastic/app_icon.png")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_map_to_real_files() {
        assert_eq!(device_file(43), "heltec-v3.svg");
        assert_eq!(device_file(4), "tbeam.svg");
        assert_eq!(device_file(50), "t-deck.svg");
        assert_eq!(device_file(110), "heltec_v4.svg");
        assert_eq!(device_file(132), "heltec_v4.svg");
        assert_eq!(device_file(134), "station-g2.svg");
        assert_eq!(device_file(0), "unknown.svg");
        assert_eq!(device_file(9999), "unknown.svg");
    }

    #[test]
    fn every_mapped_file_is_bundled() {
        for (name, bytes) in FILES {
            assert!(!bytes.is_empty(), "{name} is empty");
        }
        assert!(FILES.iter().any(|(name, _)| *name == "unknown.svg"));
        for id in 0..=255 {
            let file = device_file(id);
            assert!(
                FILES.iter().any(|(name, _)| *name == file),
                "model {id} -> {file} is not bundled"
            );
        }
    }
}
