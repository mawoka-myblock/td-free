use defmt::{error, info, unwrap, warn};
use embassy_time::Timer;
use heapless::Vec;
use micromath::F32Ext;

use crate::{
    helpers::RGBMultipliers,
    tasks::{leds::set_led_brightness, sensors::VEML3328},
};

static RGB_WHITE_BALANCE_SAMPLE_COUNT: usize = 10;
static RGB_WHITE_BALANCE_SAMPLE_DELAY: usize = 55;

pub async fn take_rgb_white_balance_calibration<'d>(v33: &mut VEML3328<'d>) -> (u16, u16, u16) {
    info!(
        "Starting comprehensive RGB white balance calibration with {} samples",
        RGB_WHITE_BALANCE_SAMPLE_COUNT
    );

    // Take calibration readings at multiple brightness levels to account for non-linearity
    let brightness_levels = [25, 50, 75]; // Different LED brightness levels
    let mut all_r_readings: Vec<u16, { RGB_WHITE_BALANCE_SAMPLE_COUNT * 3 }> = Vec::new();
    let mut all_g_readings: Vec<u16, { RGB_WHITE_BALANCE_SAMPLE_COUNT * 3 }> = Vec::new();
    let mut all_b_readings: Vec<u16, { RGB_WHITE_BALANCE_SAMPLE_COUNT * 3 }> = Vec::new();
    let mut all_clear_readings: Vec<u16, { RGB_WHITE_BALANCE_SAMPLE_COUNT * 3 }> = Vec::new();

    for &brightness in &brightness_levels {
        info!("Taking calibration readings at {} brightness", brightness);

        set_led_brightness(brightness);

        // Wait for LED to stabilize
        Timer::after_millis(300).await;

        let mut r_readings: Vec<u16, RGB_WHITE_BALANCE_SAMPLE_COUNT> = Vec::new();
        let mut g_readings: Vec<u16, RGB_WHITE_BALANCE_SAMPLE_COUNT> = Vec::new();
        let mut b_readings: Vec<u16, RGB_WHITE_BALANCE_SAMPLE_COUNT> = Vec::new();
        let mut clear_readings: Vec<u16, RGB_WHITE_BALANCE_SAMPLE_COUNT> = Vec::new();

        for i in 0..RGB_WHITE_BALANCE_SAMPLE_COUNT {
            match (
                v33.read_red(),
                v33.read_green(),
                v33.read_blue(),
                v33.read_clear(),
            ) {
                (Ok(r), Ok(g), Ok(b), Ok(clear)) => {
                    warn!(
                        "Brightness {}% sample {}: R={}, G={}, B={}, Clear={}",
                        brightness, i, r, g, b, clear
                    );
                    unwrap!(r_readings.push(r));
                    unwrap!(g_readings.push(g));
                    unwrap!(b_readings.push(b));
                    unwrap!(clear_readings.push(clear));
                }
                (r_result, g_result, b_result, clear_result) => {
                    warn!(
                        "Failed to read RGB sensor - R: {:?}, G: {:?}, B: {:?}, Clear: {:?}",
                        r_result, g_result, b_result, clear_result
                    );
                    continue;
                }
            }
            Timer::after_millis(RGB_WHITE_BALANCE_SAMPLE_DELAY as u64).await;
        }

        // Add readings from this brightness level to overall collection
        all_r_readings.extend(r_readings);
        all_g_readings.extend(g_readings);
        all_b_readings.extend(b_readings);
        all_clear_readings.extend(clear_readings);
    }

    if all_r_readings.is_empty() {
        error!("No valid RGB readings obtained during white balance, using default values");
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

    info!(
        "RGB white balance raw medians: R={}, G={}, B={}, Clear={}",
        r_median, g_median, b_median, clear_median
    );
    info!(
        "Spectral response ratios (relative to Green): R={}, B={}",
        r_ratio, b_ratio
    );
    info!(
        "Color temperature corrected: R={}, G={}, B={}",
        corrected_r, corrected_g, corrected_b
    );

    // Return color temperature corrected values
    (corrected_r, corrected_g, corrected_b)
}

pub fn spectral_correction_from_rgb(r: u16, g: u16) -> f32 {
    let r_g_ratio = r as f32 / g.max(1) as f32;

    if r_g_ratio > 1.0 {
        // Magenta/red-dominant: R/G=1.0 → 1.0x, R/G=2.25 → 5.28x
        1.0 + (r_g_ratio - 1.0) / 1.25 * 4.28
    } else {
        1.0
    }
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
        r, g, b, r_correction, g_correction, b_correction, r_final, g_final, b_final
    );

    (r_final, g_final, b_final)
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
