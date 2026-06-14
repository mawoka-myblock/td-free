use embassy_time::Timer;
use embedded_io_async::{Read, Write};
use esp_hal::{
    Async,
    usb_serial_jtag::{UsbSerialJtagRx, UsbSerialJtagTx},
};

use crate::{CLIENT_CONNECTED, MEASUREMENT_DATA_WATCH};

#[embassy_executor::task]
pub async fn handle_serial_task(
    mut tx: UsbSerialJtagTx<'static, Async>,
    mut rx: UsbSerialJtagRx<'static, Async>,
) {
    let mut buf = [0u8; 64];
    let mut cmd: heapless::Vec<u8, 128> = heapless::Vec::new();

    let mut mm_data_sub = MEASUREMENT_DATA_WATCH.receiver().unwrap();

    // Wait for "connect", reply "ready\n"
    // Then wait for "P" or "HF" before starting measurements
    let mut connected = false;
    let mut triggered = false;

    while !triggered {
        match rx.read(&mut buf).await {
            Ok(n) => {
                for &b in &buf[..n] {
                    if b == b'\n' {
                        let line = core::str::from_utf8(cmd.as_slice()).unwrap_or("").trim();
                        match line {
                            "connect" => {
                                tx.write_all(b"ready\n").await.ok();
                                tx.flush().await.ok();
                                connected = true;
                            }
                            "version" => {
                                tx.write_all(b"result, TD1 Version: V1.0.4, StatusScreen Version: V1.0.4,Comms Version: V1.0.4, startUp Version: V1.0.4\n").await.ok();
                                tx.flush().await.ok();
                            }
                            "P" | "HF" if connected => {
                                tx.write_all(b"connected to HF unlicensed\n").await.ok();
                                tx.flush().await.ok();
                                triggered = true;
                            }
                            _ => {}
                        }
                        cmd.clear();
                    } else {
                        let _ = cmd.push(b);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    CLIENT_CONNECTED.sender().send(true);

    // Client is connected and measurement triggered — start streaming CSV
    loop {
        let data = match mm_data_sub.get().await {
            Some(d) => d,
            None => {
                Timer::after_millis(200).await;
                continue;
            }
        };

        let mut line_buf = [0u8; 64];
        let line = format_csv_line(data.td, data.hex_color.as_deref(), &mut line_buf);
        tx.write_all(line).await.ok();
        tx.flush().await.ok();

        embassy_time::Timer::after_millis(100).await;
    }
}

/// Formats a usize into a decimal ASCII byte slice using a caller-provided buffer.
fn format_usize<'a>(mut n: usize, buf: &'a mut [u8; 20]) -> &'a [u8] {
    if n == 0 {
        buf[19] = b'0';
        return &buf[19..];
    }
    let mut i = 20;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    &buf[i..]
}

/// Formats a CSV line into buf. Returns the written slice.
/// Output: "0,0,0,0,<td>,<color>\n"
fn format_csv_line<'a>(td: f32, color: Option<&str>, buf: &'a mut [u8; 64]) -> &'a [u8] {
    // Write prefix
    let prefix = b"0,0,0,0,";
    buf[..prefix.len()].copy_from_slice(prefix);
    let mut pos = prefix.len();

    // Write TD float (simple fixed-point: integer.fraction)
    let td_int = td as i32;
    let td_frac = ((td - td_int as f32).abs() * 100.0) as u32;

    let mut tmp = [0u8; 20];
    let int_str = format_usize(td_int.unsigned_abs() as usize, &mut tmp);
    if td < 0.0 {
        buf[pos] = b'-';
        pos += 1;
    }
    buf[pos..pos + int_str.len()].copy_from_slice(int_str);
    pos += int_str.len();
    buf[pos] = b'.';
    pos += 1;
    // Two decimal digits
    buf[pos] = b'0' + (td_frac / 10) as u8;
    buf[pos + 1] = b'0' + (td_frac % 10) as u8;
    pos += 2;

    buf[pos] = b',';
    pos += 1;

    // Write color
    if let Some(color) = color {
        let color_bytes = color.as_bytes();
        buf[pos..pos + color_bytes.len()].copy_from_slice(color_bytes);
        pos += color_bytes.len();
    }
    buf[pos] = b'\n';
    pos += 1;

    &buf[..pos]
}
