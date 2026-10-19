#[cfg(feature = "defmt")]
use defmt::Format;
use heapless::Vec;
use micromath::F32Ext;
use serde::{Deserialize, Serialize};

use crate::{RGBMultipliers, optics};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct CalibrationCommand {
    pub target_r: u8,
    pub target_g: u8,
    pub target_b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub enum AutoCalibrateError {
    NoSignal,
}

/// Calibrate the color multipliers so the currently measured transmittance
/// (or spectral reading) maps to the target color.
pub fn auto_calibrate_gray_reference(
    command_data: CalibrationCommand,
    transmittance: (f32, f32, f32),
    td_value: f32,
    mut rgb_multipliers: RGBMultipliers,
) -> Result<RGBMultipliers, AutoCalibrateError> {
    if transmittance.0 <= 0.0 && transmittance.1 <= 0.0 && transmittance.2 <= 0.0 {
        return Err(AutoCalibrateError::NoSignal);
    }

    rgb_multipliers.td_reference = td_value;

    let target = (
        command_data.target_r,
        command_data.target_g,
        command_data.target_b,
    );

    let optimized_brightness = optimize_brightness(transmittance, target, rgb_multipliers, 100);
    rgb_multipliers.brightness = optimized_brightness;

    let (optimized_red, optimized_green, optimized_blue) =
        optimize_rgb_channels(transmittance, target, rgb_multipliers, 100);

    Ok(RGBMultipliers {
        red: optimized_red,
        green: optimized_green,
        blue: optimized_blue,
        brightness: optimized_brightness,
        td_reference: td_value,
        reference_r: command_data.target_r,
        reference_g: command_data.target_g,
        reference_b: command_data.target_b,
    })
}

/// Calibrate color multipliers directly from a dual-band `SpectralReadings` measurement.
pub fn auto_calibrate_spectral(
    command_data: CalibrationCommand,
    readings: &optics::SpectralReadings,
    baselines: &optics::ChannelBaselines,
    td_value: f32,
    mut rgb_multipliers: RGBMultipliers,
) -> Result<RGBMultipliers, AutoCalibrateError> {
    let base_rgb = optics::base_color_from_spectral(readings, baselines);
    if base_rgb == (0, 0, 0) {
        return Err(AutoCalibrateError::NoSignal);
    }

    rgb_multipliers.td_reference = td_value;
    rgb_multipliers.brightness = 1.0;
    rgb_multipliers.red = if base_rgb.0 > 0 {
        (command_data.target_r as f32 / base_rgb.0 as f32).clamp(0.05, 5.0)
    } else {
        1.0
    };
    rgb_multipliers.green = if base_rgb.1 > 0 {
        (command_data.target_g as f32 / base_rgb.1 as f32).clamp(0.05, 5.0)
    } else {
        1.0
    };
    rgb_multipliers.blue = if base_rgb.2 > 0 {
        (command_data.target_b as f32 / base_rgb.2 as f32).clamp(0.05, 5.0)
    } else {
        1.0
    };
    rgb_multipliers.reference_r = command_data.target_r;
    rgb_multipliers.reference_g = command_data.target_g;
    rgb_multipliers.reference_b = command_data.target_b;

    Ok(rgb_multipliers)
}

pub fn optimize_brightness(
    transmittance: (f32, f32, f32),
    target_color: (u8, u8, u8),
    mut multipliers: RGBMultipliers,
    max_iterations: usize,
) -> f32 {
    let mut best_brightness = multipliers.brightness;
    let current_result = optics::apply_rgb_multipliers(transmittance, &multipliers);
    let mut best_distance = calculate_rgb_distance(current_result, target_color);

    let step_size = 0.02;
    let mut step_direction = 0;

    for _iteration in 0..max_iterations {
        let mut improved = false;
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

            let test_result = optics::apply_rgb_multipliers(transmittance, &test_multipliers);
            let test_distance = calculate_rgb_distance(test_result, target_color);

            if test_distance < best_distance {
                best_distance = test_distance;
                best_brightness = test_brightness;
                step_direction = direction as i32;
                improved = true;
                break;
            }
        }

        if improved {
            multipliers.brightness = best_brightness;
        } else {
            break;
        }
    }

    best_brightness
}

pub fn calculate_rgb_distance(color1: (u8, u8, u8), color2: (u8, u8, u8)) -> f32 {
    let dr = color1.0 as f32 - color2.0 as f32;
    let dg = color1.1 as f32 - color2.1 as f32;
    let db = color1.2 as f32 - color2.2 as f32;
    F32Ext::sqrt(dr * dr + dg * dg + db * db)
}

pub fn optimize_rgb_channels(
    transmittance: (f32, f32, f32),
    target_color: (u8, u8, u8),
    mut multipliers: RGBMultipliers,
    max_iterations: usize,
) -> (f32, f32, f32) {
    let step_size = 0.01;
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

        let mut step_direction = 0;
        let initial_result = optics::apply_rgb_multipliers(transmittance, &multipliers);
        let initial_channel_value = match *channel {
            "red" => initial_result.0,
            "green" => initial_result.1,
            "blue" => initial_result.2,
            _ => 127,
        };

        let mut best_channel_distance =
            F32Ext::abs(initial_channel_value as f32 - target_channel_value as f32);

        for _iteration in 0..max_iterations {
            let mut improved = false;
            let mut directions: Vec<f32, 2> = Vec::new();

            if step_direction == 0 {
                let _ = directions.push(1.0);
                let _ = directions.push(-1.0);
            } else {
                let _ = directions.push(step_direction as f32);
            }

            for &direction in &directions {
                let test_value = (best_value + direction * step_size).clamp(0.2, 3.0);
                let mut test_multipliers = multipliers;
                match *channel {
                    "red" => test_multipliers.red = test_value,
                    "green" => test_multipliers.green = test_value,
                    "blue" => test_multipliers.blue = test_value,
                    _ => {}
                }

                let test_result = optics::apply_rgb_multipliers(transmittance, &test_multipliers);
                let test_channel_value = match *channel {
                    "red" => test_result.0,
                    "green" => test_result.1,
                    "blue" => test_result.2,
                    _ => 127,
                };

                let test_channel_distance =
                    F32Ext::abs(test_channel_value as f32 - target_channel_value as f32);

                if test_channel_distance < best_channel_distance {
                    best_channel_distance = test_channel_distance;
                    best_value = test_value;
                    step_direction = direction as i32;
                    improved = true;
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
                break;
            }
        }
    }

    (multipliers.red, multipliers.green, multipliers.blue)
}
