//! Headless smoke test for a real node over any transport.
//!
//! Usage: `cargo run -p mt-core --example serial_probe -- <address> [seconds]`
//!
//! `<address>` accepts a bare BLE MAC (`10:BD:A3:5B:07:F9`), a serial path
//! (`/dev/ttyACM0`) or a TCP host (`192.168.1.42`), as well as the explicit
//! `x`/`s`/`t` prefix forms. Connects, completes the handshake, prints a
//! summary and disconnects.

use std::io::BufRead;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mt_core::{ConnectionState, CoreCommand, CoreConfig, CoreEvent, DeviceAddress, spawn_core};
use tokio::time::timeout;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let target = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/ttyACM0".into());
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let address = match DeviceAddress::parse_manual(&target) {
        Ok(address) => address,
        Err(error) => {
            eprintln!("invalid address {target:?}: {error}");
            return;
        }
    };

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let data_dir = std::env::temp_dir().join(format!("mt-probe-{nanos}"));

    let core = spawn_core(CoreConfig {
        data_dir: data_dir.clone(),
        auto_reconnect: false,
        handshake_timeout: Duration::from_secs(15),
        ..CoreConfig::default()
    });
    let mut events = core.subscribe();

    // Let the user type a BLE pairing passkey read off the device's screen.
    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<String>(4);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if line_tx.blocking_send(line).is_err() {
                break;
            }
        }
    });

    println!("connecting to {address} …");
    core.connect(address).await.unwrap();

    let mut my_node = None;
    let mut nodes = 0usize;
    let mut channels = 0usize;
    let mut configs = 0usize;
    let mut module_configs = 0usize;
    let mut region = String::new();
    let mut bluetooth = String::new();
    let mut firmware = String::new();
    let mut hw_model = 0i32;
    let mut ready = false;

    let _ = timeout(Duration::from_secs(seconds), async {
        loop {
            match events.recv().await {
                Ok(CoreEvent::Connection(ConnectionState::Connected { node_num, .. })) => {
                    my_node = Some(node_num);
                    ready = true;
                }
                Ok(CoreEvent::Node(node)) => {
                    nodes += 1;
                    if let Some(user) = &node.user {
                        hw_model = user.hw_model;
                    }
                }
                Ok(CoreEvent::Channel(_)) => channels += 1,
                Ok(CoreEvent::Config(config)) => {
                    configs += 1;
                    if let Some(meshtastic_protobufs::meshtastic::config::PayloadVariant::Lora(
                        lora,
                    )) = &config.payload_variant
                    {
                        region = format!("{:?}", lora.region);
                    }
                    if let Some(meshtastic_protobufs::meshtastic::config::PayloadVariant::Bluetooth(
                        bt,
                    )) = &config.payload_variant
                    {
                        let mode = meshtastic_protobufs::meshtastic::config::bluetooth_config::PairingMode::try_from(bt.mode)
                            .map(|m| format!("{m:?}"))
                            .unwrap_or_else(|_| bt.mode.to_string());
                        bluetooth = format!("{mode} fixed_pin={}", bt.fixed_pin);
                    }
                }
                Ok(CoreEvent::ModuleConfig(_)) => module_configs += 1,
                Ok(CoreEvent::Metadata(m)) => firmware = m.firmware_version.clone(),
                Ok(CoreEvent::Error(e)) => eprintln!("core error: {e}"),
                Ok(CoreEvent::BlePairingRequest { address }) => {
                    println!(
                        "PAIRING: {address} is showing a passkey. Enter it and press Enter:"
                    );
                    if let Some(line) = line_rx.recv().await {
                        match line.trim().parse::<u32>() {
                            Ok(passkey) => {
                                let _ = core.dispatch(CoreCommand::SubmitBlePasskey(passkey)).await;
                                println!("passkey submitted");
                            }
                            Err(_) => println!("not a passkey; skipping"),
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
            if ready {
                break;
            }
        }
    })
    .await;

    if ready {
        println!("HANDSHAKE OK");
        println!(
            "  node      : {}",
            my_node.map(|n| format!("!{n:08x}")).unwrap_or_default()
        );
        println!("  firmware  : {firmware}");
        println!("  channels  : {channels}");
        println!("  nodes     : {nodes}");
        println!("  hw_model  : {hw_model}");
        println!("  configs   : {configs} device, {module_configs} module");
        println!("  region    : {region}");
        println!("  bluetooth : {bluetooth}");
    } else {
        println!("handshake did NOT complete within {seconds}s");
    }

    let _ = core.dispatch(CoreCommand::Disconnect).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = std::fs::remove_dir_all(data_dir);
}
