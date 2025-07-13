use std::sync::Arc;
use std::sync::atomic::Ordering;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use esp_idf_svc::hal::usb_serial::UsbSerialDriver;
use esp_idf_svc::io::Write;
use std::sync::atomic::AtomicBool;

use crate::helpers;
use crate::helpers::readings::LAST_MEASUREMENT;

pub async fn serial_connection(conn: &mut UsbSerialDriver<'static>) -> Result<(), anyhow::Error> {
    let mut buffer = [0u8; 64];
    let trigger_measurement = Arc::new(AtomicBool::new(false));
    let trigger_clone = trigger_measurement.clone();
    let channel = Channel::<NoopRawMutex, String, 1>::new();
    let recv = channel.receiver();
    let send = channel.sender();

    let conn_loop = async {
        loop {
            if trigger_measurement.load(Ordering::SeqCst) {
                embassy_time::Timer::after_millis(300).await;
            } else {
                embassy_time::Timer::after_millis(600).await;
            }
            let n = match conn.read(&mut buffer, 50) {
                Ok(n) if n > 0 => Some(n),
                _ => None,
            };
            if let Some(n) = n {
                let received: &str = core::str::from_utf8(&buffer[..n]).unwrap_or("").trim();
                match received {
                    "connect" => {
                        conn.write(b"ready\n", 100).unwrap();
                    }
                    "P" | "HF" => {
                        trigger_measurement.store(true, Ordering::SeqCst);
                        conn.write(b"connected to HF unlicensed\n", 100).unwrap();
                    }
                    "version" => {
                        conn.write(b"result, TD1 Version: V1.0.4, StatusScreen Version: V1.0.4,Comms Version: V1.0.4, startUp Version: V1.0.4\n", 100).unwrap();
                    }
                    _ => {}
                }
                conn.flush().unwrap();
            } else if trigger_measurement.load(Ordering::SeqCst)
                && let Ok(msg) = recv.try_receive()
            {
                conn.write(msg.as_bytes(), 500).unwrap();
                conn.flush().unwrap();
                recv.clear();
            }
            continue;
        }
    };

    let measurement_loop = async {
        loop {
            if !trigger_clone.load(Ordering::SeqCst) {
                embassy_time::Timer::after_millis(500).await;
                continue;
            }

            let last_measurement = LAST_MEASUREMENT.lock().unwrap();
            if let Some(measurement) = &*last_measurement
                && measurement.filament_inserted
            {
                let message = format!(
                    "{},,,,{:.1},{:02X}{:02X}{:02X}\n",
                    helpers::generate_random_11_digit_number(),
                    measurement.td_value,
                    measurement.r,
                    measurement.g,
                    measurement.b
                );
                send.send(message).await;
            }

            // The frontend is responsible for polling and updating the values.
            // We just need to wait and check periodically.
            embassy_time::Timer::after_millis(300).await;
        }
    };

    embassy_futures::join::join(measurement_loop, conn_loop).await;
    Ok(())
}
