//! Scan for discoverable devices (BLE, serial, mDNS) and print them.
//!
//! Run with: `cargo run -p mt-transport --example scan -- 15`

use std::time::Duration;

#[tokio::main]
async fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(15);

    let discovery = mt_transport::spawn_discovery();
    let mut events = discovery.subscribe();
    discovery.start_ble_scan().await;
    println!("scanning for {seconds}s...");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
            Ok(Ok(mt_transport::DiscoveryEvent::DevicesUpdated(devices))) => {
                println!("-- {} devices --", devices.len());
                for device in devices {
                    println!(
                        "  {:<32} {:<28} [{}]",
                        device.name, device.address, device.detail
                    );
                }
            }
            Ok(Ok(mt_transport::DiscoveryEvent::Error(error))) => println!("error: {error}"),
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }

    discovery.stop_ble_scan().await;
}
