use defmt::{Debug2Format, error, info, unwrap, warn};
use embassy_time::Timer;
use heapless::Vec;
use micromath::F32Ext;

use crate::tasks::{leds::set_led_brightness, sensors::VEML7700};

const BASELINE_SAMPLE_COUNT: usize = 20;

pub async fn take_baseline_reading<'d>(veml: &mut VEML7700<'d>) -> f32 {
    let sample_delay = 100u32; // Increased delay to ensure fresh readings
    let mut readings: Vec<f32, BASELINE_SAMPLE_COUNT> = Vec::new();

    for _ in 0..BASELINE_SAMPLE_COUNT {
        let clr = match veml.read_lux() {
            Ok(d) => d,
            Err(e) => {
                error!("{:?}", Debug2Format(&e));
                veml.disable().unwrap();
                Timer::after_millis(100).await;
                veml.enable().unwrap();
                Timer::after_millis(1000).await;
                continue;
            }
        };
        let reading = clr as f32;
        info!("Reading: {}", reading);
        readings.push(reading).unwrap();
        Timer::after_millis(sample_delay as u64).await;
    }

    if readings.is_empty() {
        return 0.0; // Avoid divide by zero or panics later
    }

    // Calculate mean and std deviation
    let mean = readings.iter().copied().sum::<f32>() / readings.len() as f32;
    let std =
        (readings.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / readings.len() as f32).sqrt();

    // Filter out outliers
    let mut filtered: Vec<f32, BASELINE_SAMPLE_COUNT> = readings
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

    info!(
        "Baseline calculation: mean={}, std={}, median={}",
        mean, std, median
    );
    median
}

pub async fn is_filament_inserted<'d>(
    veml: &mut VEML7700<'d>,
    dark_baseline_reading: f32,
    threshold: f32,
) -> (bool, f32) {
    let mut detection_readings: Vec<f32, 3> = Vec::new();
    set_led_brightness(25);
    for i in 0..3 {
        let current_reading = {
            match veml.read_lux() {
                Ok(d) => d,
                Err(e) => {
                    error!(
                        "Failed to read sensor (attempt {}): {:?}",
                        i + 1,
                        Debug2Format(&e)
                    );
                    if i == 2 {
                        // If all 3 attempts failed, return None
                        return (false, 0.0);
                    }
                    continue;
                }
            }
        };
        unwrap!(detection_readings.push(current_reading));

        if i < 2 {
            embassy_time::Timer::after_millis(100).await;
        }
    }
    detection_readings.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_reading = detection_readings[1]; // Middle value (median of 3)
    // let mean = detection_readings.iter().sum::<f32>() / 3.0;
    // let variance = detection_readings
    //     .iter()
    //     .map(|x| (x - mean).powi(2))
    //     .sum::<f32>()
    //     / 3.0;
    // let std_dev = variance.sqrt();

    // if std_dev < 0.01 && median_reading > 10.0 {
    //     warn!(
    //         "VEML7700 readings very similar (std_dev: {}) - sensor might need more time",
    //         std_dev
    //     );
    // }

    let current_threshold = dark_baseline_reading - (1.0 - threshold) * dark_baseline_reading;
    if median_reading > current_threshold {
        (false, median_reading)
    } else {
        (true, median_reading)
    }
}
