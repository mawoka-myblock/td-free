use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use esp_idf_svc::hal::usb_serial::UsbSerialDriver;
use esp_idf_svc::io::Write;
use std::sync::atomic::AtomicBool;

use crate::led::set_led;
use crate::{LedType, helpers};

pub async fn serial_connection(
    conn: &mut UsbSerialDriver<'static>,
    ws2812: Arc<Mutex<LedType<'static>>>,
    ext_channel: Arc<Channel<NoopRawMutex, Option<String>, 1>>,
) -> Result<(), anyhow::Error> {
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
            set_led(ws2812.clone(), 100, 30, 255);

            ext_channel.send(None).await;
            embassy_time::Timer::after_millis(100).await;
            let res = ext_channel.receive().await.unwrap_or_default();
            if res == "no_filament" {
                embassy_time::Timer::after_millis(500).await;
                continue;
            }
            let measurement = res.split(",").collect::<Vec<&str>>();
            let message = format!(
                "{},,,,{},000000\n",
                helpers::generate_random_11_digit_number(),
                measurement[0],
            );
            send.send(message).await;

            // The frontend is responsible for polling and updating the values.
            // We just need to wait and check periodically.
            loop {
                embassy_time::Timer::after_millis(1000).await;
                ext_channel.send(None).await;
                let res = ext_channel.receive().await;
                if res.is_some() && res.unwrap() == "no_filament" {
                    continue;
                }
                break;
            }
        }
    };

    embassy_futures::join::join(measurement_loop, conn_loop).await;
    Ok(())
}
