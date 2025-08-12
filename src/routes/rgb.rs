use edge_http::io::server::Connection;
use embedded_io_async::{Read, Write};
use picoserve::response::{Body, HeadersIter, Response, StatusCode};

use crate::{
    AppState, WsHandler, WsHandlerError,
    helpers::{
        nvs::{RGBMultipliers, save_rgb_multipliers},
        rgb::{apply_complete_color_correction, optimize_brightness, optimize_rgb_channels},
    },
};
use edge_http::io::Error as EdgeError;

pub async fn get_rgb_multipliers(state: AppState) -> Response<impl HeadersIter, impl Body> {
    let multipliers = state.saved_rgb_multipliers.lock().unwrap();
    let json_response = format!(
        r#"{{"red": {:.2}, "green": {:.2}, "blue": {:.2}, "brightness": {:.2}, "td_reference": {:.2}, "reference_r": {}, "reference_g": {}, "reference_b": {}, "rgb_disabled": true}}"#,
        multipliers.red,
        multipliers.green,
        multipliers.blue,
        multipliers.brightness,
        multipliers.td_reference,
        multipliers.reference_r,
        multipliers.reference_g,
        multipliers.reference_b
    );
    drop(multipliers);
    Response::new(StatusCode::OK, json_response).with_header("Content-Type", "application/json")
}
#[derive(serde::Deserialize)]
pub struct AutoCalibrateGrayInput {
    reference_r: Option<u8>,
    reference_g: Option<u8>,
    reference_b: Option<u8>,
}
pub async fn auto_calibrate_gray_reference(
    state: AppState,
    data: AutoCalibrateGrayInput,
) -> Response<impl HeadersIter, impl Body> {
    if state.rgb.is_none() {
        return Response::new(
            StatusCode::NOT_FOUND,
            r#"{"status": "disabled", "message": "RGB Disabled"}"#,
        )
        .with_header("Content-Type", "application/json");
    }
    let rgb_d = state.rgb.clone().unwrap();
    log::info!("Starting optimization-based color calibration...");

    // Parse reference color from request body or use current saved values
    let (target_r, target_g, target_b) =
        if data.reference_b.is_some() && data.reference_g.is_some() && data.reference_r.is_some() {
            (
                data.reference_r.unwrap(),
                data.reference_g.unwrap(),
                data.reference_b.unwrap(),
            )
        } else {
            let multipliers = state.saved_rgb_multipliers.lock().unwrap();
            (
                multipliers.reference_r,
                multipliers.reference_g,
                multipliers.reference_b,
            )
        };

    log::info!("Calibrating to target color: RGB({target_r}, {target_g}, {target_b})");

    // Get the current median lux for this material using the buffer median
    let current_lux = {
        let buffer = state.lux_buffer.lock().unwrap();
        let median = buffer.median();
        drop(buffer);
        if let Some(current_reading) = median {
            current_reading
        } else {
            return Response::new(
                StatusCode::BAD_REQUEST,
                r#"{"status": "error", "message": "No valid lux buffer for normalization"}"#,
            )
            .with_header("Content-Type", "application/json");
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
        return Response::new(
            StatusCode::BAD_REQUEST,
            r#"{"status": "error", "message": "No valid color readings available"}"#,
        )
        .with_header("Content-Type", "application/json");
    }

    log::info!("Current raw median readings: R={current_r}, G={current_g}, B={current_b}");

    // Get current multipliers as starting point for optimization
    let mut current_multipliers = {
        let multipliers = state.saved_rgb_multipliers.lock().unwrap();
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
        let mut multipliers = state.saved_rgb_multipliers.lock().unwrap();
        *multipliers = new_multipliers;
    }

    // Save to NVS
    match save_rgb_multipliers(new_multipliers, state.nvs.as_ref().clone()) {
        Ok(_) => {
            let body = format!(
                r#"{{"status": "success", "red": {optimized_red:.2}, "green": {optimized_green:.2}, "blue": {optimized_blue:.2}, "brightness": {optimized_brightness:.2}, "td_reference": {current_lux:.2}}}"#,
            );
            return Response::new(StatusCode::OK, body)
                .with_header("Content-Type", "application/json");
        }
        Err(e) => {
            log::error!("Failed to save optimized multipliers: {e:?}");
            return Response::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"status": "error", "message": "Failed to save calibration"}"#,
            )
            .with_header("Content-Type", "application/json");
        }
    }
}

#[derive(serde::Deserialize)]
pub struct SetRgbMultiplierJsonData {
    red: f32,
    green: f32,
    blue: f32,
    brightness: f32,
    reference_r: u8,
    reference_g: u8,
    reference_b: u8,
}

pub async fn set_rgb_multipliers(
    state: AppState,
    data: SetRgbMultiplierJsonData,
) -> Response<impl HeadersIter, impl Body> {
    // Clamp values to reasonable ranges
    let red = data.red.clamp(0.1, 5.0);
    let green = data.green.clamp(0.1, 5.0);
    let blue = data.blue.clamp(0.1, 5.0);
    let brightness = data.brightness.clamp(0.1, 5.0);

    // Get current TD reference to preserve it
    let current_td_reference = {
        let multipliers = state.saved_rgb_multipliers.lock().unwrap();
        multipliers.td_reference
    };

    let new_multipliers = RGBMultipliers {
        red,
        green,
        blue,
        brightness,
        td_reference: current_td_reference,
        reference_r: data.reference_r,
        reference_g: data.reference_g,
        reference_b: data.reference_b,
    };

    // Update the in-memory multipliers
    {
        let mut multipliers = state.saved_rgb_multipliers.lock().unwrap();
        *multipliers = new_multipliers;
    }

    // Save to NVS
    return match save_rgb_multipliers(new_multipliers, state.nvs.as_ref().clone()) {
        Ok(_) => Response::new(StatusCode::OK, r#"{"status": "saved"}"#)
            .with_header("Content-Type", "application/json"),
        Err(e) => {
            log::error!("Failed to save RGB multipliers: {e:?}");
            Response::new(StatusCode::OK, r#"{"status": "error"}"#)
                .with_header("Content-Type", "application/json")
        }
    };
}
