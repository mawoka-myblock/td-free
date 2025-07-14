use edge_http::io::server::Connection;
use embedded_io_async::{Read, Write};

use crate::{
    WsHandler, WsHandlerError,
    helpers::{
        nvs::{RGBMultipliers, save_rgb_multipliers},
        rgb::{apply_complete_color_correction, optimize_brightness, optimize_rgb_channels},
    },
};
use edge_http::io::Error as EdgeError;

impl WsHandler<'_> {
    pub async fn get_rgb_multipliers<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        let multipliers = self.saved_rgb_multipliers.lock().unwrap();
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
        conn.initiate_response(200, None, &[("Content-Type", "application/json")])
            .await?;
        conn.write_all(json_response.as_bytes()).await?;
        Ok(())
    }

    pub async fn set_rgb_multipliers<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        // Read the request body
        let mut buffer = [0u8; 256];
        let mut total_read = 0;

        // Read the Content-Length header to know how much data to expect
        let content_length = conn
            .headers()?
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .unwrap_or(0);

        if content_length > buffer.len() {
            conn.initiate_response(400, Some("Payload too large"), &[])
                .await?;
            return Ok(());
        }

        while total_read < content_length {
            let bytes_read = conn.read(&mut buffer[total_read..content_length]).await?;
            if bytes_read == 0 {
                break;
            }
            total_read += bytes_read;
        }

        let body_str = match str::from_utf8(&buffer[..total_read]) {
            Ok(s) => s,
            Err(_) => {
                conn.initiate_response(400, Some("Invalid UTF-8"), &[])
                    .await?;
                return Ok(());
            }
        };

        // Simple JSON parsing for the extended format
        let mut red = 1.0f32;
        let mut green = 1.0f32;
        let mut blue = 1.0f32;
        let mut brightness = 1.0f32;
        let mut reference_r = 127u8;
        let mut reference_g = 127u8;
        let mut reference_b = 127u8;

        for part in body_str.split(',') {
            let part = part.trim().trim_matches('{').trim_matches('}');
            if let Some((key, value)) = part.split_once(':') {
                let key = key.trim().trim_matches('"');
                let value = value.trim();

                match key {
                    "red" => red = value.parse().unwrap_or(1.0),
                    "green" => green = value.parse().unwrap_or(1.0),
                    "blue" => blue = value.parse().unwrap_or(1.0),
                    "brightness" => brightness = value.parse().unwrap_or(1.0),
                    "reference_r" => reference_r = value.parse().unwrap_or(127),
                    "reference_g" => reference_g = value.parse().unwrap_or(127),
                    "reference_b" => reference_b = value.parse().unwrap_or(127),
                    _ => {}
                }
            }
        }

        // Clamp values to reasonable ranges
        red = red.clamp(0.1, 5.0);
        green = green.clamp(0.1, 5.0);
        blue = blue.clamp(0.1, 5.0);
        brightness = brightness.clamp(0.1, 5.0);

        // Get current TD reference to preserve it
        let current_td_reference = {
            let multipliers = self.saved_rgb_multipliers.lock().unwrap();
            multipliers.td_reference
        };

        let new_multipliers = RGBMultipliers {
            red,
            green,
            blue,
            brightness,
            td_reference: current_td_reference,
            reference_r,
            reference_g,
            reference_b,
        };

        // Update the in-memory multipliers
        {
            let mut multipliers = self.saved_rgb_multipliers.lock().unwrap();
            *multipliers = new_multipliers;
        }

        // Save to NVS
        match save_rgb_multipliers(new_multipliers, self.nvs.as_ref().clone()) {
            Ok(_) => {
                conn.initiate_response(200, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(br#"{"status": "saved"}"#).await?;
            }
            Err(e) => {
                log::error!("Failed to save RGB multipliers: {e:?}");
                conn.initiate_response(500, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(br#"{"status": "error"}"#).await?;
            }
        }

        Ok(())
    }

    pub async fn auto_calibrate_gray_reference<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        if self.rgb.is_none() {
            conn.initiate_response(404, None, &[("Content-Type", "application/json")])
                .await?;
            conn.write_all(br#"{"status": "disabled", "message": "RGB Disabled"}"#)
                .await?;
            return Ok(());
        }
        let rgb_d = self.rgb.clone().unwrap();
        log::info!("Starting optimization-based color calibration...");

        // Read the request body to get reference color
        let mut buffer = [0u8; 256];
        let mut total_read = 0;

        let content_length = conn
            .headers()?
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .unwrap_or(0);

        if content_length > 0 && content_length <= buffer.len() {
            while total_read < content_length {
                let bytes_read = conn.read(&mut buffer[total_read..content_length]).await?;
                if bytes_read == 0 {
                    break;
                }
                total_read += bytes_read;
            }
        }

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
            let multipliers = self.saved_rgb_multipliers.lock().unwrap();
            (
                multipliers.reference_r,
                multipliers.reference_g,
                multipliers.reference_b,
            )
        };

        log::info!("Calibrating to target color: RGB({target_r}, {target_g}, {target_b})");

        // Get the current median lux for this material using the buffer median
        let current_lux = {
            let buffer = self.lux_buffer.lock().unwrap();
            let median = buffer.median();
            drop(buffer);
            if let Some(current_reading) = median {
                current_reading
            } else {
                conn.initiate_response(400, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(
                    br#"{"status": "error", "message": "No valid lux buffer for normalization"}"#,
                )
                .await?;
                return Ok(());
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
            conn.initiate_response(400, None, &[("Content-Type", "application/json")])
                .await?;
            conn.write_all(
                br#"{"status": "error", "message": "No valid color readings available"}"#,
            )
            .await?;
            return Ok(());
        }

        log::info!("Current raw median readings: R={current_r}, G={current_g}, B={current_b}");

        // Get current multipliers as starting point for optimization
        let mut current_multipliers = {
            let multipliers = self.saved_rgb_multipliers.lock().unwrap();
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
            let mut multipliers = self.saved_rgb_multipliers.lock().unwrap();
            *multipliers = new_multipliers;
        }

        // Save to NVS
        match save_rgb_multipliers(new_multipliers, self.nvs.as_ref().clone()) {
            Ok(_) => {
                let response = format!(
                    r#"{{"status": "success", "red": {optimized_red:.2}, "green": {optimized_green:.2}, "blue": {optimized_blue:.2}, "brightness": {optimized_brightness:.2}, "td_reference": {current_lux:.2}}}"#,
                );
                conn.initiate_response(200, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(response.as_bytes()).await?;
            }
            Err(e) => {
                log::error!("Failed to save optimized multipliers: {e:?}");
                conn.initiate_response(500, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(br#"{"status": "error", "message": "Failed to save calibration"}"#)
                    .await?;
            }
        }

        Ok(())
    }
}
