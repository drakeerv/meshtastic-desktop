//! Read or change a node's BLE pairing mode over serial.
//!
//! Usage: `cargo run -p mt-core --example ble_pin -- <address> <random|fixed|none> [pin]`
//!
//! Handy for testing the in-app pairing flow against a known passkey, then
//! restoring `random`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use meshtastic_protobufs::meshtastic::{Config, config};
use mt_core::{ConnectionState, CoreCommand, CoreConfig, CoreEvent, DeviceAddress, spawn_core};
use tokio::time::timeout;

#[tokio::main]
async fn main() {
    let address = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/ttyACM0".into());
    let mode = std::env::args().nth(2).unwrap_or_else(|| "random".into());
    let pin: u32 = std::env::args()
        .nth(3)
        .and_then(|p| p.parse().ok())
        .unwrap_or(123456);

    let pairing = match mode.as_str() {
        "fixed" => config::bluetooth_config::PairingMode::FixedPin,
        "none" => config::bluetooth_config::PairingMode::NoPin,
        _ => config::bluetooth_config::PairingMode::RandomPin,
    };

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let data_dir = std::env::temp_dir().join(format!("mt-blepin-{nanos}"));

    let core = spawn_core(CoreConfig {
        data_dir: data_dir.clone(),
        auto_reconnect: false,
        handshake_timeout: Duration::from_secs(15),
        ..CoreConfig::default()
    });
    let mut events = core.subscribe();
    let address = DeviceAddress::parse_manual(&address).expect("address");

    core.connect(address).await.unwrap();
    let _ = timeout(Duration::from_secs(20), async {
        loop {
            match events.recv().await {
                Ok(CoreEvent::Connection(ConnectionState::Connected { .. })) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
    .await;

    let config = Config {
        payload_variant: Some(config::PayloadVariant::Bluetooth(config::BluetoothConfig {
            enabled: true,
            mode: pairing as i32,
            fixed_pin: pin,
        })),
    };
    if mode == "reboot" {
        core.dispatch(CoreCommand::Reboot {
            dest: u32::MAX,
            seconds: 2,
        })
        .await
        .unwrap();
        println!("rebooting the node…");
    } else {
        core.dispatch(CoreCommand::SetConfig(Box::new(config)))
            .await
            .unwrap();
        println!("set ble pairing mode to {mode} (pin {pin}); waiting for reboot…");
    }
    tokio::time::sleep(Duration::from_secs(8)).await;

    let _ = core.dispatch(CoreCommand::Disconnect).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = std::fs::remove_dir_all(data_dir);
}
