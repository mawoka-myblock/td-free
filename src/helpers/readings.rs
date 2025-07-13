use std::sync::{Arc, Mutex};

use embedded_hal::pwm::SetDutyCycle;
use esp_idf_svc::hal::ledc::LedcDriver;
use once_cell::sync::Lazy;
use veml7700::Veml7700;

use crate::{
    LedType,
    helpers::{
        HardwareI2cInstance, NvsData, RGBMultipliers, SimpleBitBangI2cInstance,
        median_buffer::{RunningMedianBuffer, RunningMedianBufferU16},
        rgb::{apply_rgb_multipliers, apply_spectral_response_correction},
    },
    led::set_led,
    veml3328,
    wifi::WifiEnum,
};

// Static for concurrency control and caching last result
pub static BUSY: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
pub static LAST_DATA: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[derive(Clone, Copy, Debug)]
pub struct LastMeasurement {
    pub td_value: f32,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub filament_inserted: bool,
}

pub static LAST_MEASUREMENT: Lazy<Mutex<Option<LastMeasurement>>> = Lazy::new(|| Mutex::new(None));

#[allow(clippy::too_many_arguments)]
pub async fn read_data_with_buffer(
    veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>,
    veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    rgb_white_balance: (u16, u16, u16),
    _dark_rgb_baseline: (u16, u16, u16),
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'_>>>,
    ws2812: Arc<Mutex<LedType<'_>>>,
    saved_algorithm: NvsData,
    lux_buffer: Arc<Mutex<RunningMedianBuffer>>,
    rgb_buffers: Arc<
        Mutex<(
            RunningMedianBufferU16,
            RunningMedianBufferU16,
            RunningMedianBufferU16,
        )>,
    >,
    rgb_multipliers: Arc<Mutex<RGBMultipliers>>,
) -> Option<String> {
    // We need to be under 1 seconds for this function.

    // Take quick readings for robust filament detection using median
    let mut detection_readings: Vec<f32> = Vec::with_capacity(3);

    // Only lock once and drop before reacquiring
    let current_led_brightness = {
        let led = led_light.lock().unwrap();
        led.get_duty()
    };
    log::info!("Current LED brightness: {current_led_brightness:?}");

    // Only lock again if needed, and drop immediately after
    if current_led_brightness != 25 {
        log::info!("Setting LED to fully on for filament detection");
        {
            let mut led = led_light.lock().unwrap();
            if let Err(e) = led.set_duty(25) {
                log::error!("Failed to set LED duty cycle: {e:?}");
                return None;
            }
        }
        embassy_time::Timer::after_millis(350).await;
    }

    for i in 0..3 {
        let current_reading = {
            let mut locked_veml = veml.lock().unwrap();
            match locked_veml.read_lux() {
                Ok(d) => d,
                Err(e) => {
                    log::error!("Failed to read sensor (attempt {}): {:?}", i + 1, e);
                    if i == 2 {
                        // If all 3 attempts failed, return None
                        return None;
                    }
                    continue;
                }
            }
        };
        detection_readings.push(current_reading);

        if i < 2 {
            embassy_time::Timer::after_millis(100).await;
        }
    }
    // worst case time = 300 + 2 * 100 = 500ms

    // Calculate median of the 3 readings for filament detection
    detection_readings.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_reading = detection_readings[1]; // Middle value (median of 3)

    // Calculate variance to check if readings are diverse enough
    let mean = detection_readings.iter().sum::<f32>() / 3.0;
    let variance = detection_readings
        .iter()
        .map(|x| (x - mean).powi(2))
        .sum::<f32>()
        / 3.0;
    let std_dev = variance.sqrt();

    log::info!(
        "Filament detection readings: [{:.2}, {:.2}, {:.2}] -> median: {:.2}, std_dev: {:.3}",
        detection_readings[0],
        detection_readings[1],
        detection_readings[2],
        median_reading,
        std_dev
    );

    // Warn if readings are too similar (might indicate sensor issue)
    if std_dev < 0.01 && median_reading > 10.0 {
        log::warn!(
            "VEML7700 readings very similar (std_dev: {std_dev:.3}) - sensor might need more time"
        );
    }

    let brightness_diff = dark_baseline_reading;
    let current_threshold =
        dark_baseline_reading - (1.0 - saved_algorithm.threshold) * brightness_diff;
    log::info!(
        "Detection threshold check: {median_reading:.2} (threshold: {current_threshold:.2})"
    );

    // Use median reading for filament detection
    if median_reading > current_threshold {
        // Clear buffers when no filament is detected
        {
            let mut buffer = lux_buffer.lock().unwrap();
            buffer.clear();
        }
        {
            let mut buffers = rgb_buffers.lock().unwrap();
            buffers.0.clear();
            buffers.1.clear();
            buffers.2.clear();
        }

        // Update last measurement cache
        {
            let mut last_measurement = LAST_MEASUREMENT.lock().unwrap();
            if let Some(meas) = last_measurement.as_mut() {
                meas.filament_inserted = false;
            } else {
                *last_measurement = Some(LastMeasurement {
                    td_value: 0.0,
                    r: 0,
                    g: 0,
                    b: 0,
                    filament_inserted: false,
                });
            }
        }

        let wifi_stat = wifi_status.lock().unwrap();
        match *wifi_stat {
            WifiEnum::Connected => set_led(ws2812.clone(), 0, 255, 0),
            WifiEnum::HotSpot => set_led(ws2812.clone(), 255, 0, 255),
            WifiEnum::Working => set_led(ws2812.clone(), 255, 255, 0),
        }
        return Some("no_filament".to_string());
    }

    // Filament is detected
    log::info!("Filament detected!");
    set_led(ws2812.clone(), 0, 125, 125);

    {
        let mut led = led_light.lock().unwrap();
        if let Err(e) = led.set_duty_cycle_fully_on() {
            log::error!("Failed to set LED duty cycle: {e:?}");
            return None;
        }
    }

    // Wait for LED to stabilize before taking measurements
    embassy_time::Timer::after_millis(350).await;

    // worst case time = 500 + 300 = 800ms

    // Take multiple readings for median calculation with longer delays
    let readings_per_call = 3;
    for i in 0..readings_per_call {
        // Longer delay to ensure fresh VEML7700 readings
        if i > 0 {
            embassy_time::Timer::after_millis(100).await; // Increased from 15ms to 60ms
        }

        {
            let mut locked_veml = veml.lock().unwrap();
            let mut buffer = lux_buffer.lock().unwrap();
            let lux_reading = locked_veml.read_lux().unwrap_or(0.0);
            buffer.push(lux_reading);
        }

        let mut locked_rgb = veml_rgb.lock().unwrap();
        if let (Ok(r), Ok(g), Ok(b)) = (
            locked_rgb.read_red(),
            locked_rgb.read_green(),
            locked_rgb.read_blue(),
        ) {
            log::debug!("RGB readings {}: R={}, G={}, B={}", i + 1, r, g, b);

            let mut buffers = rgb_buffers.lock().unwrap();
            buffers.0.push(r);
            buffers.1.push(g);
            buffers.2.push(b);
        }
        drop(locked_rgb); // Release lock
    }

    // worst case time = 800 + 2 * 100 = 1000ms

    // Get buffer count for confidence indicator
    let buffer_count = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.len()
    };

    // Get median values for accurate measurement
    let final_median_lux = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.median().unwrap_or(median_reading) // Fallback to detection median if buffer is empty
    };

    let (r_median_raw, g_median_raw, b_median_raw) = {
        let buffers = rgb_buffers.lock().unwrap();
        (
            buffers.0.median().unwrap_or(rgb_white_balance.0),
            buffers.1.median().unwrap_or(rgb_white_balance.1),
            buffers.2.median().unwrap_or(rgb_white_balance.2),
        )
    };

    // Calculate TD from RAW lux reading
    let td_value = (final_median_lux / baseline_reading) * 10.0;
    let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;

    log::info!(
        "Final TD value: {:.2} (raw lux: {:.2}, baseline: {:.2}, m: {:.3}, b: {:.3})",
        adjusted_td_value,
        final_median_lux,
        baseline_reading,
        saved_algorithm.m,
        saved_algorithm.b
    );

    // Read clear channel for brightness correction (RAW)
    let clear_median_raw = {
        let mut locked_rgb = veml_rgb.lock().unwrap();
        locked_rgb.read_clear().unwrap_or(rgb_white_balance.0)
    };

    log::debug!(
        "RAW median values: Lux={final_median_lux:.2}, RGB=({r_median_raw},{g_median_raw},{b_median_raw}), Clear={clear_median_raw}"
    );

    // NOW apply calibration/correction to the RAW median values
    // Step 1: Apply spectral response correction to RAW medians
    let (r_corrected, g_corrected, b_corrected) = apply_spectral_response_correction(
        r_median_raw,
        g_median_raw,
        b_median_raw,
        rgb_white_balance.0,
        rgb_white_balance.1,
        rgb_white_balance.2,
    );

    log::info!("Spectral corrected RGB: ({r_corrected},{g_corrected},{b_corrected})");

    // Step 2: Apply user RGB multipliers with lux-based brightness adjustment to corrected values
    let (r_final, g_final, b_final) = {
        let multipliers = rgb_multipliers.lock().unwrap();
        apply_rgb_multipliers(
            r_corrected,
            g_corrected,
            b_corrected,
            final_median_lux,
            &multipliers,
        )
    };

    // Update last measurement cache
    {
        let mut last_measurement = LAST_MEASUREMENT.lock().unwrap();
        *last_measurement = Some(LastMeasurement {
            td_value: adjusted_td_value,
            r: r_final,
            g: g_final,
            b: b_final,
            filament_inserted: true,
        });
    }

    // Create hex color string with corrected values
    let hex_color = format!("#{r_final:02X}{g_final:02X}{b_final:02X}");

    let ws_message = format!("{adjusted_td_value:.2},{hex_color},{buffer_count}");

    // Log buffer status and detailed color information
    let (lux_len, rgb_len) = {
        let lux_buf = lux_buffer.lock().unwrap();
        let rgb_buf = rgb_buffers.lock().unwrap();
        (lux_buf.len(), rgb_buf.0.len())
    };

    log::info!(
        "Reading: {:.2}, RGB: {} (medians from {} lux, {} RGB samples, confidence: {}), Raw RGB: ({},{},{}), Final RGB: ({},{},{}) - Baseline: {:.2}, Lux: {}, Clear: {}",
        adjusted_td_value,
        hex_color,
        lux_len,
        rgb_len,
        buffer_count,
        r_median_raw,
        g_median_raw,
        b_median_raw,
        r_final,
        g_final,
        b_final,
        saved_algorithm.b,
        final_median_lux,
        clear_median_raw
    );

    Some(ws_message)
}
