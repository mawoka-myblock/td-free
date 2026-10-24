use defmt::{error, info, unwrap, warn};
use embassy_time::Timer;
use heapless::Vec;

use crate::tasks::{leds::set_led_brightness, sensors::VEML3328};

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
