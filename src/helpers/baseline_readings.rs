use std::sync::{Arc, Mutex};

use embedded_hal::delay::DelayNs;
use esp_idf_svc::hal::{delay::FreeRtos, ledc::LedcDriver};
use veml7700::Veml7700;

use crate::veml3328;

use super::bitbang_i2c::{HardwareI2cInstance, SimpleBitBangI2cInstance};

pub fn take_rgb_white_balance_calibration(
    veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>,
    led_light: Arc<Mutex<LedcDriver<'_>>>,
) -> (u16, u16, u16) {
    let sample_count = 10; // Increased samples for better accuracy
    let sample_delay = 55u32;

    log::info!("Starting comprehensive RGB white balance calibration with {sample_count} samples");

    // Take calibration readings at multiple brightness levels to account for non-linearity
    let brightness_levels = [25, 50, 75]; // Different LED brightness levels
    let mut all_r_readings: Vec<u16> = Vec::new();
    let mut all_g_readings: Vec<u16> = Vec::new();
    let mut all_b_readings: Vec<u16> = Vec::new();
    let mut all_clear_readings: Vec<u16> = Vec::new();

    for &brightness in &brightness_levels {
        log::info!("Taking calibration readings at {brightness} brightness");

        {
            let mut led = led_light.lock().unwrap();
            led.set_duty(brightness).unwrap();
        }

        // Wait for LED to stabilize
        FreeRtos.delay_ms(300);

        let mut r_readings: Vec<u16> = Vec::new();
        let mut g_readings: Vec<u16> = Vec::new();
        let mut b_readings: Vec<u16> = Vec::new();
        let mut clear_readings: Vec<u16> = Vec::new();

        for i in 0..sample_count {
            let mut locked_veml = veml_rgb.lock().unwrap();
            match (
                locked_veml.read_red(),
                locked_veml.read_green(),
                locked_veml.read_blue(),
                locked_veml.read_clear(),
            ) {
                (Ok(r), Ok(g), Ok(b), Ok(clear)) => {
                    log::debug!(
                        "Brightness {brightness}% sample {i}: R={r}, G={g}, B={b}, Clear={clear}"
                    );
                    r_readings.push(r);
                    g_readings.push(g);
                    b_readings.push(b);
                    clear_readings.push(clear);
                }
                (r_result, g_result, b_result, clear_result) => {
                    log::warn!(
                        "Failed to read RGB sensor - R: {r_result:?}, G: {g_result:?}, B: {b_result:?}, Clear: {clear_result:?}"
                    );
                    continue;
                }
            }
            drop(locked_veml);
            FreeRtos.delay_ms(sample_delay);
        }

        // Add readings from this brightness level to overall collection
        all_r_readings.extend(r_readings);
        all_g_readings.extend(g_readings);
        all_b_readings.extend(b_readings);
        all_clear_readings.extend(clear_readings);
    }

    if all_r_readings.is_empty() {
        log::error!("No valid RGB readings obtained during white balance, using default values");
        return (1000, 1000, 1000);
    }

    // Calculate median values across all brightness levels
    all_r_readings.sort();
    all_g_readings.sort();
    all_b_readings.sort();
    all_clear_readings.sort();

    let r_median = all_r_readings[all_r_readings.len() / 2];
    let g_median = all_g_readings[all_g_readings.len() / 2];
    let b_median = all_b_readings[all_b_readings.len() / 2];
    let clear_median = if !all_clear_readings.is_empty() {
        all_clear_readings[all_clear_readings.len() / 2]
    } else {
        r_median + g_median + b_median
    };

    // Calculate spectral response ratios for proper white balance
    // Assume green channel as reference (typically most sensitive in visible range)
    let g_ref = g_median as f32;
    let r_ratio = r_median as f32 / g_ref;
    let b_ratio = b_median as f32 / g_ref;

    // Apply color temperature correction based on my measurements for the sensor
    let led_color_temp_correction_r = 1.00;
    let led_color_temp_correction_g = 1.00;
    let led_color_temp_correction_b = 1.00;

    let corrected_r = (r_median as f32 * led_color_temp_correction_r) as u16;
    let corrected_g = (g_median as f32 * led_color_temp_correction_g) as u16;
    let corrected_b = (b_median as f32 * led_color_temp_correction_b) as u16;

    log::info!(
        "RGB white balance raw medians: R={r_median}, G={g_median}, B={b_median}, Clear={clear_median}"
    );
    log::info!("Spectral response ratios (relative to Green): R={r_ratio:.3}, B={b_ratio:.3}");
    log::info!("Color temperature corrected: R={corrected_r}, G={corrected_g}, B={corrected_b}");

    // Return color temperature corrected values
    (corrected_r, corrected_g, corrected_b)
}

pub fn take_baseline_reading(veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>) -> f32 {
    let sample_count = 20;
    let sample_delay = 100u32; // Increased delay to ensure fresh readings
    let mut readings: Vec<f32> = Vec::with_capacity(sample_count as usize);

    for _ in 0..sample_count {
        let mut locked_veml = veml.lock().unwrap();
        let clr = match locked_veml.read_lux() {
            Ok(d) => d,
            Err(e) => {
                log::error!("{e:?}");
                veml.lock().unwrap().disable().unwrap();
                FreeRtos.delay_ms(100);
                veml.lock().unwrap().enable().unwrap();
                FreeRtos.delay_ms(1000);
                continue;
            }
        };
        let reading = clr as f32;
        log::info!("Reading: {reading}");
        readings.push(reading);
        drop(locked_veml); // Release lock before delay
        FreeRtos.delay_ms(sample_delay);
    }

    if readings.is_empty() {
        return 0.0; // Avoid divide by zero or panics later
    }

    // Calculate mean and std deviation
    let mean = readings.iter().copied().sum::<f32>() / readings.len() as f32;
    let std =
        (readings.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / readings.len() as f32).sqrt();

    // Filter out outliers
    let mut filtered: Vec<f32> = readings
        .into_iter()
        .filter(|v| (*v - mean).abs() <= 2.0 * std)
        .collect();

    // Calculate median from filtered data
    if filtered.is_empty() {
        return mean; // fallback to mean if all readings were filtered out
    }

    filtered.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if filtered.len().is_multiple_of(2) {
        let mid = filtered.len() / 2;
        (filtered[mid - 1] + filtered[mid]) / 2.0
    } else {
        filtered[filtered.len() / 2]
    };

    log::info!("Baseline calculation: mean={mean:.2}, std={std:.2}, median={median:.2}");
    median
}
