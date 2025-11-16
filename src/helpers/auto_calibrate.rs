use std::sync::{Arc, Mutex};

use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use log::info;

use crate::{
    RgbWsHandler,
    helpers::{
        median_buffer,
        nvs::{RGBMultipliers, save_rgb_multipliers},
        rgb::{apply_complete_color_correction, optimize_brightness, optimize_rgb_channels},
    },
};

pub fn auto_calibrate(
    rgb: Option<RgbWsHandler>,
    saved_rgb_multipliers: Arc<Mutex<RGBMultipliers>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    lux_buffer: Arc<Mutex<median_buffer::RunningMedianBuffer>>,
) -> String {
    if rgb.is_none() {
        return r#"{"status": "disabled", "message": "RGB Disabled"}"#.to_string();
    }
    let rgb_d = rgb.clone().unwrap();
    log::info!("Starting optimization-based color calibration...");

    // Read the request body to get reference color
    let buffer = [0u8; 256];
    let total_read = 0;

    // Parse reference color from request body or use current saved values
    let (target_r, target_g, target_b) = if total_read > 0 {
        let body_str = str::from_utf8(&buffer[..total_read]).unwrap_or("{}");
        let mut ref_r = 127u8;
        let mut ref_g = 127u8;
        let mut ref_b = 127u8;

        for part in body_str.split(',') {
            let part = part.trim().trim_matches('{').trim_matches('}');
            if let Some((key, value)) = part.split_once(':') {
                let key = key.trim().trim_matches('"');
                let value = value.trim();

                match key {
                    "reference_r" => ref_r = value.parse().unwrap_or(127),
                    "reference_g" => ref_g = value.parse().unwrap_or(127),
                    "reference_b" => ref_b = value.parse().unwrap_or(127),
                    _ => {}
                }
            }
        }
        (ref_r, ref_g, ref_b)
    } else {
        let multipliers = saved_rgb_multipliers.lock().unwrap();
        (
            multipliers.reference_r,
            multipliers.reference_g,
            multipliers.reference_b,
        )
    };

    log::info!("Calibrating to target color: RGB({target_r}, {target_g}, {target_b})");

    // Get the current median lux for this material using the buffer median
    let current_lux = {
        let buffer = lux_buffer.lock().unwrap();
        info!("Lux buffer calibrate reading {buffer:?}");
        let median = buffer.median();
        drop(buffer);
        if let Some(current_reading) = median {
            current_reading
        } else {
            return r#"{"status": "error", "message": "No valid lux buffer for normalization"}"#
                .to_string();
        }
    };

    // Take current RAW median reading from the buffers
    let (current_r, current_g, current_b) = {
        let buffers = rgb_d.rgb_buffers.lock().unwrap();
        (
            buffers.0.median().unwrap_or(0),
            buffers.1.median().unwrap_or(0),
            buffers.2.median().unwrap_or(0),
        )
    };

    if current_r == 0 && current_g == 0 && current_b == 0 {
        return r#"{"status": "error", "message": "No valid color readings available"}"#
            .to_string();
    }

    log::info!("Current raw median readings: R={current_r}, G={current_g}, B={current_b}");

    // Get current multipliers as starting point for optimization
    let mut current_multipliers = {
        let multipliers = saved_rgb_multipliers.lock().unwrap();
        *multipliers
    };

    //set the current multiplier td to lux. TODO: Rename this field
    current_multipliers.td_reference = current_lux;

    log::info!(
        "Starting optimization from current multipliers: R={:.3}, G={:.3}, B={:.3}, Brightness={:.3}",
        current_multipliers.red,
        current_multipliers.green,
        current_multipliers.blue,
        current_multipliers.brightness
    );

    // Step 1: Optimize brightness to minimize overall RGB distance
    let optimized_brightness = optimize_brightness(
        (current_r, current_g, current_b),
        (target_r, target_g, target_b),
        rgb_d.rgb_baseline,
        current_lux,
        current_multipliers,
        100,
    );

    current_multipliers.brightness = optimized_brightness;

    // Step 2: Fine-tune individual RGB channels
    let (optimized_red, optimized_green, optimized_blue) = optimize_rgb_channels(
        (current_r, current_g, current_b),
        (target_r, target_g, target_b),
        rgb_d.rgb_baseline,
        current_lux,
        current_multipliers,
        100,
    );

    current_multipliers.red = optimized_red;
    current_multipliers.green = optimized_green;
    current_multipliers.blue = optimized_blue;

    log::info!(
        "Optimization result: R={optimized_red:.3}, G={optimized_green:.3}, B={optimized_blue:.3}, Brightness={optimized_brightness:.3}"
    );

    // Verify the optimization result
    let verify_result = apply_complete_color_correction(
        (current_r, current_g, current_b).0,
        (current_r, current_g, current_b).1,
        (current_r, current_g, current_b).2,
        rgb_d.rgb_baseline,
        current_lux,
        &current_multipliers,
    );

    log::info!(
        "Verification - Target: RGB({},{},{}), Optimized result: RGB({},{},{})",
        target_r,
        target_g,
        target_b,
        verify_result.0,
        verify_result.1,
        verify_result.2
    );

    // Set the multipliers with the optimized values
    let new_multipliers = RGBMultipliers {
        red: optimized_red,
        green: optimized_green,
        blue: optimized_blue,
        brightness: optimized_brightness,
        td_reference: current_lux, // Not used for normalization anymore, but keep for compatibility
        reference_r: target_r,
        reference_g: target_g,
        reference_b: target_b,
    };

    log::info!(
        "Setting new TD reference: {:.2} (was {:.2})",
        current_lux,
        current_multipliers.td_reference
    );

    // Update the in-memory multipliers
    {
        let mut multipliers = saved_rgb_multipliers.lock().unwrap();
        *multipliers = new_multipliers;
    }

    // Save to NVS
    match save_rgb_multipliers(new_multipliers, nvs.as_ref().clone()) {
        Ok(_) => {
            format!(
                r#"{{"status": "success", "red": {optimized_red:.2}, "green": {optimized_green:.2}, "blue": {optimized_blue:.2}, "brightness": {optimized_brightness:.2}, "td_reference": {current_lux:.2}}}"#,
            )
        }
        Err(_) => r#"{"status": "error", "message": "Failed to save calibration"}"#.to_string(),
    }
}
