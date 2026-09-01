use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, bleuuid::uuid_from_u16};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::StreamExt;
use std::error::Error;
use std::time::Duration;
use tokio::time::sleep;

const ACCUMULATED_TORQUE_PRESENT: u16 = 0x0004; // bit 2
const WHEEL_REVOLUTION_DATA_PRESENT: u16 = 0x0010; // bit 4

#[derive(serde::Serialize)]
struct Snapshot {
    power: i16,
    torque: Option<u16>,
    revs: Option<(u32, u16)>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let manager: Manager = Manager::new().await?;
    let devices: Vec<Adapter> = manager.adapters().await?;
    let central = devices.into_iter().next().ok_or("No devices found.")?;

    let ftms_uuid = uuid_from_u16(0x1826);
    let cycling_power_uuid = uuid_from_u16(0x1818);

    central.start_scan(ScanFilter::default()).await?;
    println!("scan started.");
    sleep(Duration::from_secs(5)).await;
    let peripherals = central.peripherals().await?;

    println!("finding trainer");
    let mut trainer: Option<Peripheral> = None;
    for peripheral in &peripherals {
        if let Some(props) = peripheral.properties().await? {
            if props.services.contains(&ftms_uuid) || props.services.contains(&cycling_power_uuid) {
                trainer = Some(peripheral.clone());
                break;
            }
        }
    }
    let trainer = trainer.ok_or("No FTMS trainer found.")?;

    println!("connecting to trainer");
    trainer.connect().await?;
    trainer.discover_services().await?;

    let cycling_power_measurement_uuid = uuid_from_u16(0x2A63);
    let power_char = trainer
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == cycling_power_measurement_uuid)
        .ok_or("No cycling measurement UUID found.")?;

    trainer.subscribe(&power_char).await?;
    let mut notification_stream = trainer.notifications().await?;
    while let Some(data) = notification_stream.next().await {
        let mut offset = 0;

        let flags = u16::from_le_bytes([data.value[offset], data.value[offset + 1]]);
        offset += 2;

        let torque_present = (flags & ACCUMULATED_TORQUE_PRESENT) != 0;
        let revs_present = (flags & WHEEL_REVOLUTION_DATA_PRESENT) != 0;

        let instantaneous_power = i16::from_le_bytes([data.value[offset], data.value[offset + 1]]);
        offset += 2;

        let torque = if torque_present {
            let t = u16::from_le_bytes([data.value[offset], data.value[offset + 1]]);
            offset += 2;
            Some(t)
        } else {
            None
        };

        let revs = if revs_present {
            let count = u32::from_le_bytes([
                data.value[offset],
                data.value[offset + 1],
                data.value[offset + 2],
                data.value[offset + 3],
            ]);
            offset += 4;
            let time = u16::from_le_bytes([data.value[offset], data.value[offset + 1]]);
            offset += 2;
            Some((count, time))
        } else {
            None
        };

        let snapshot = Snapshot {
            power: instantaneous_power,
            torque,
            revs,
        };
        let snapshot_json = serde_json::to_string(&snapshot)?;
        println!("{snapshot_json}");
    }
    Ok(())
}
