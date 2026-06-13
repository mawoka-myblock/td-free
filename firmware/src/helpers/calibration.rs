use defmt::{Format, debug, info};
use heapless::Vec;
use micromath::F32Ext;
use serde::{Deserialize, Serialize};

use crate::helpers::{RGBMultipliers, median_buffer::RunningMedianBuffer};

#[derive(Debug, Format, Clone, Copy, Deserialize, Serialize)]
pub struct CalibrationCommand {
    pub target_r: u8,
    pub target_g: u8,
    pub target_b: u8,
}

#[derive(Debug, Format)]
pub enum AutoCalibrateError {
    RgbBufferEmpty,
}

pub async fn auto_calibrate_gray_reference(
    command_data: CalibrationCommand,
    final_median_lux: f32,
    rgb_bufs: &(
        RunningMedianBuffer<100>,
        RunningMedianBuffer<100>,
        RunningMedianBuffer<100>,
    ),
    rgb_wb: (u16, u16, u16),
    mut rgb_multipliers: RGBMultipliers,
) -> Result<RGBMultipliers, AutoCalibrateError> {
    info!("Starting optimization-based color calibration...");

    info!(
        "Calibrating to target color: RGB({}, {}, {})",
        command_data.target_r, command_data.target_g, command_data.target_b
    );

    // Take current RAW median reading from the buffers
    let (current_r, current_g, current_b) = (
        rgb_bufs.0.median().unwrap_or(0.0),
        rgb_bufs.1.median().unwrap_or(0.0),
        rgb_bufs.2.median().unwrap_or(0.0),
    );

    if current_r == 0.0 && current_g == 0.0 && current_b == 0.0 {
        return Err(AutoCalibrateError::RgbBufferEmpty);
    }

    info!(
        "Current raw median readings: R={}, G={}, B={}",
        current_r, current_g, current_b
    );

    // Get current multipliers as starting point for optimization

    //set the current multiplier td to lux. TODO: Rename this field
    rgb_multipliers.td_reference = final_median_lux;

    info!(
        "Starting optimization from current multipliers: R={}, G={}, B={}, Brightness={}",
        rgb_multipliers.red,
        rgb_multipliers.green,
        rgb_multipliers.blue,
        rgb_multipliers.brightness
    );

    // Step 1: Optimize brightness to minimize overall RGB distance
    let optimized_brightness = optimize_brightness(
        (current_r as u16, current_g as u16, current_b as u16),
        (
            command_data.target_r,
            command_data.target_g,
            command_data.target_b,
        ),
        rgb_wb,
        final_median_lux,
        rgb_multipliers,
        100,
    );

    rgb_multipliers.brightness = optimized_brightness;

    // Step 2: Fine-tune individual RGB channels
    let (optimized_red, optimized_green, optimized_blue) = optimize_rgb_channels(
        (current_r as u16, current_g as u16, current_b as u16),
        (
            command_data.target_r,
            command_data.target_g,
            command_data.target_b,
        ),
        rgb_wb,
        final_median_lux,
        rgb_multipliers,
        100,
    );

    rgb_multipliers.red = optimized_red;
    rgb_multipliers.green = optimized_green;
    rgb_multipliers.blue = optimized_blue;

    info!(
        "Optimization result: R={}, G={}, B={}, Brightness={}",
        optimized_red, optimized_green, optimized_blue, optimized_brightness
    );

    // Verify the optimization result
    let verify_result = apply_complete_color_correction(
        (current_r, current_g, current_b).0 as u16,
        (current_r, current_g, current_b).1 as u16,
        (current_r, current_g, current_b).2 as u16,
        rgb_wb,
        final_median_lux,
        &rgb_multipliers,
    );

    info!(
        "Verification - Target: RGB({},{},{}), Optimized result: RGB({},{},{})",
        command_data.target_r,
        command_data.target_g,
        command_data.target_b,
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
        td_reference: final_median_lux, // Not used for normalization anymore, but keep for compatibility
        reference_r: command_data.target_r,
        reference_g: command_data.target_g,
        reference_b: command_data.target_b,
    };

    info!(
        "Setting new TD reference: {} (was {})",
        final_median_lux, rgb_multipliers.td_reference
    );

    // Update the in-memory multipliers
    Ok(new_multipliers)
}

pub fn optimize_brightness(
    raw_color: (u16, u16, u16),
    target_color: (u8, u8, u8),
    white_balance: (u16, u16, u16),
    current_lux: f32,
    mut multipliers: RGBMultipliers,
    max_iterations: usize,
) -> f32 {
    let mut best_brightness = multipliers.brightness;

    // Current distance
    let current_result = apply_complete_color_correction(
        raw_color.0,
        raw_color.1,
        raw_color.2,
        white_balance,
        current_lux,
        &multipliers,
    );
    let mut current_distance = calculate_rgb_distance(current_result, target_color);
    let mut best_distance = current_distance;

    info!(
        "Brightness optimization start: brightness={}, distance={}",
        multipliers.brightness, current_distance
    );

    let step_size = 0.02; // 5% steps
    let mut step_direction = 0; // 0=unknown, 1=increase, -1=decrease

    for iteration in 0..max_iterations {
        let mut improved = false;

        // Try both directions if we don't know the direction yet
        let mut directions: Vec<f32, 2> = Vec::new();

        if step_direction == 0 {
            let _ = directions.push(1.0);
            let _ = directions.push(-1.0);
        } else {
            let _ = directions.push(step_direction as f32);
        }

        for &direction in &directions {
            let test_brightness = (multipliers.brightness + direction * step_size).clamp(0.1, 3.0);

            let mut test_multipliers = multipliers;
            test_multipliers.brightness = test_brightness;

            let test_result = apply_complete_color_correction(
                raw_color.0,
                raw_color.1,
                raw_color.2,
                white_balance,
                current_lux,
                &test_multipliers,
            );

            let test_distance = calculate_rgb_distance(test_result, target_color);

            if test_distance < best_distance {
                best_distance = test_distance;
                best_brightness = test_brightness;
                step_direction = direction as i32;
                improved = true;

                debug!(
                    "Brightness iter {}: {} -> distance {} (improved)",
                    iteration, test_brightness, test_distance
                );
                break;
            }
        }

        if improved {
            multipliers.brightness = best_brightness;
            current_distance = best_distance;
        } else {
            // No improvement found, stop
            break;
        }
    }

    info!(
        "Brightness optimization complete: {} -> {}, distance: {} -> {}",
        multipliers.brightness, best_brightness, current_distance, best_distance
    );

    best_brightness
}

pub fn apply_spectral_response_correction(
    r: u16,
    g: u16,
    b: u16,
    wb_r: u16,
    wb_g: u16,
    wb_b: u16,
) -> (u8, u8, u8) {
    // Calculate relative sensitivities from white balance calibration
    let total_wb = wb_r as f32 + wb_g as f32 + wb_b as f32;
    if total_wb == 0.0 {
        return (128, 128, 128); // Gray fallback
    }

    // Normalize white balance values to get relative channel sensitivities
    let wb_r_norm = wb_r as f32 / total_wb;
    let wb_g_norm = wb_g as f32 / total_wb;
    let wb_b_norm = wb_b as f32 / total_wb;

    // Calculate correction factors - use green as reference (typically most stable)
    let target_balance = 1.0 / 3.0; // Equal RGB in white light
    let r_correction = target_balance / wb_r_norm;
    let g_correction = target_balance / wb_g_norm;
    let b_correction = target_balance / wb_b_norm;

    // Apply spectral response correction
    let r_corrected = (r as f32 * r_correction).round();
    let g_corrected = (g as f32 * g_correction).round();
    let b_corrected = (b as f32 * b_correction).round();

    // Find maximum to normalize to 0-255 range
    let max_corrected = r_corrected.max(g_corrected).max(b_corrected);
    let (r_final, g_final, b_final) = if max_corrected > 255.0 {
        let scale = 255.0 / max_corrected;
        (
            (r_corrected * scale).round().min(255.0).max(0.0) as u8,
            (g_corrected * scale).round().min(255.0).max(0.0) as u8,
            (b_corrected * scale).round().min(255.0).max(0.0) as u8,
        )
    } else {
        (
            r_corrected.min(255.0).max(0.0) as u8,
            g_corrected.min(255.0).max(0.0) as u8,
            b_corrected.min(255.0).max(0.0) as u8,
        )
    };

    info!(
        "Spectral correction: Raw({},{},{}) -> WB factors({},{},{}) -> Final({},{},{})",
        r, g, b, r_correction, g_correction, g_correction, r_final, g_final, b_final
    );

    (r_final, g_final, b_final)
}

fn calculate_rgb_distance(color1: (u8, u8, u8), color2: (u8, u8, u8)) -> f32 {
    let dr = color1.0 as f32 - color2.0 as f32;
    let dg = color1.1 as f32 - color2.1 as f32;
    let db = color1.2 as f32 - color2.2 as f32;
    (dr * dr + dg * dg + db * db).sqrt()
}

fn apply_complete_color_correction(
    raw_r: u16,
    raw_g: u16,
    raw_b: u16,
    white_balance: (u16, u16, u16),
    current_lux: f32,
    multipliers: &RGBMultipliers,
) -> (u8, u8, u8) {
    // Step 1: Apply spectral response correction
    let (corrected_r, corrected_g, corrected_b) = apply_spectral_response_correction(
        raw_r,
        raw_g,
        raw_b,
        white_balance.0,
        white_balance.1,
        white_balance.2,
    );

    // Step 2: Apply RGB multipliers with lux-based brightness normalization
    apply_rgb_multipliers(
        corrected_r,
        corrected_g,
        corrected_b,
        current_lux,
        multipliers,
    )
}

pub fn apply_rgb_multipliers(
    r: u8,
    g: u8,
    b: u8,
    current_lux: f32,
    multipliers: &RGBMultipliers,
) -> (u8, u8, u8) {
    // Avoid division by zero
    let safe_current_lux = current_lux.max(1.0);

    // Calculate normalization factor to reach target lux
    let normalization_factor = (safe_current_lux / multipliers.td_reference).clamp(0.01, 10.0);

    //hardcoded multipliers that work as a good baseline
    let r_baseline = 0.85;
    let g_baseline = 0.93;
    let b_baseline = 1.26;
    let brightness_baseline = 1.15;

    // Apply color multipliers
    let r_color_corrected = r as f32 * multipliers.red * r_baseline;
    let g_color_corrected = g as f32 * multipliers.green * g_baseline;
    let b_color_corrected = b as f32 * multipliers.blue * b_baseline;

    // Apply total brightness (user brightness * normalization)
    let total_brightness = multipliers.brightness * normalization_factor * brightness_baseline;

    let r_final = (r_color_corrected * total_brightness)
        .round()
        .clamp(0.0, 255.0) as u8;
    let g_final = (g_color_corrected * total_brightness)
        .round()
        .clamp(0.0, 255.0) as u8;
    let b_final = (b_color_corrected * total_brightness)
        .round()
        .clamp(0.0, 255.0) as u8;

    info!(
        "Lux-normalized correction: ({},{},{}) * Color({},{},{}) * Brightness({}) * Norm({}) = ({},{},{})",
        r,
        g,
        b,
        multipliers.red,
        multipliers.green,
        multipliers.blue,
        multipliers.brightness,
        normalization_factor,
        r_final,
        g_final,
        b_final
    );

    (r_final, g_final, b_final)
}

pub fn optimize_rgb_channels(
    raw_color: (u16, u16, u16),
    target_color: (u8, u8, u8),
    white_balance: (u16, u16, u16),
    current_lux: f32,
    mut multipliers: RGBMultipliers,
    max_iterations: usize,
) -> (f32, f32, f32) {
    let step_size = 0.01; // 2% steps for fine-tuning

    // Optimize each channel independently
    let channels = ["red", "green", "blue"];

    for channel in &channels {
        let mut best_value = match *channel {
            "red" => multipliers.red,
            "green" => multipliers.green,
            "blue" => multipliers.blue,
            _ => 1.0,
        };

        let target_channel_value = match *channel {
            "red" => target_color.0,
            "green" => target_color.1,
            "blue" => target_color.2,
            _ => 127,
        };

        let mut step_direction = 0; // 0=unknown, 1=increase, -1=decrease

        // Get initial channel distance
        let initial_result = apply_complete_color_correction(
            raw_color.0,
            raw_color.1,
            raw_color.2,
            white_balance,
            current_lux,
            &multipliers,
        );

        let initial_channel_value = match *channel {
            "red" => initial_result.0,
            "green" => initial_result.1,
            "blue" => initial_result.2,
            _ => 127,
        };

        let mut best_channel_distance =
            (initial_channel_value as f32 - target_channel_value as f32).abs();

        info!(
            "{} channel optimization start: multiplier={}, current={}, target={}, distance={}",
            channel, best_value, initial_channel_value, target_channel_value, best_channel_distance
        );

        for iteration in 0..max_iterations {
            let mut improved = false;

            let mut directions: Vec<f32, 2> = Vec::new();

            if step_direction == 0 {
                let _ = directions.push(1.0);
                let _ = directions.push(-1.0);
            } else {
                let _ = directions.push(step_direction as f32);
            }

            for &direction in &directions {
                let test_value = (best_value + direction * step_size).clamp(0.5, 2.0);

                let mut test_multipliers = multipliers;
                match *channel {
                    "red" => test_multipliers.red = test_value,
                    "green" => test_multipliers.green = test_value,
                    "blue" => test_multipliers.blue = test_value,
                    _ => {}
                }

                let test_result = apply_complete_color_correction(
                    raw_color.0,
                    raw_color.1,
                    raw_color.2,
                    white_balance,
                    current_lux,
                    &test_multipliers,
                );

                let test_channel_value = match *channel {
                    "red" => test_result.0,
                    "green" => test_result.1,
                    "blue" => test_result.2,
                    _ => 127,
                };

                let test_channel_distance =
                    (test_channel_value as f32 - target_channel_value as f32).abs();

                if test_channel_distance < best_channel_distance {
                    best_channel_distance = test_channel_distance;
                    best_value = test_value;
                    step_direction = direction as i32;
                    improved = true;

                    debug!(
                        "{} iter {}: {} -> value {} distance {} (improved)",
                        channel, iteration, test_value, test_channel_value, test_channel_distance
                    );
                    break;
                }
            }

            if improved {
                match *channel {
                    "red" => multipliers.red = best_value,
                    "green" => multipliers.green = best_value,
                    "blue" => multipliers.blue = best_value,
                    _ => {}
                }
            } else {
                // No improvement found for this channel, move to next
                break;
            }
        }

        info!(
            "{} channel optimization complete: {}, distance: {}",
            channel, best_value, best_channel_distance
        );
    }

    (multipliers.red, multipliers.green, multipliers.blue)
}
