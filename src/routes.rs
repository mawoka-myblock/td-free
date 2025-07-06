use std::{
    borrow::Cow,
    collections::HashMap,
    str,
    sync::{Arc, Mutex},
    fmt::{Debug, Display},
};

use edge_http::io::server::{Connection, Handler};
use edge_http::Method as EdgeMethod;
use embedded_hal::pwm::SetDutyCycle;
use embedded_io_async::{Read, Write};
use embedded_svc::http::client::Client;
use esp_idf_svc::{
    hal::{ledc::LedcDriver, reset},
    http::{client::EspHttpConnection, Method},
    io::Write as _,
};
use log::{error, info};
use url::Url;
use veml7700::Veml7700;

use once_cell::sync::Lazy;

use crate::{
    helpers::{self, read_spoolman_data, NvsData, HardwareI2cInstance, SimpleBitBangI2cInstance, RGBMultipliers},
    led::set_led,
    median_buffer::{RunningMedianBuffer, RunningMedianBufferU16},
    serve_algo_setup_page, serve_wifi_setup_page,
    veml3328,
    wifi::{self, WifiEnum},
    EdgeError, LedType, WsHandler, WsHandlerError,
};

static INDEX_HTML: &str = include_str!("index.html");

impl WsHandler<'_> {
    pub async fn server_index_page<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        let spoolman_data = helpers::read_spoolman_data(self.nvs.as_ref().clone());
        let spoolman_available =
            match spoolman_data.0.is_some() && !spoolman_data.0.unwrap().is_empty() {
                true => "true",
                false => "false",
            };
        conn.initiate_response(200, None, &[("Content-Type", "text/html")])
            .await?;
        conn.write_all(
            INDEX_HTML
                .replace(
                    "{{VERSION}}",
                    option_env!("TD_FREE_VERSION").unwrap_or("UNKNOWN"),
                )
                .replace("{{ SPOOLMAN_AVAILABLE }}", spoolman_available)
                .as_bytes(),
        )
        .await?;
        Ok(())
    }
}

impl WsHandler<'_> {
    pub async fn wifi_route<T, const N: usize>(
        &self,
        path: &str,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        let url = Url::parse(&format!("http://google.com{}", path)).unwrap();
        let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let ssid = url_params.get("ssid");
        let password = url_params.get("password");
        if ssid.is_none() && password.is_none() {
            let saved_ssid =
                wifi::get_wifi_ssid(self.nvs.clone().as_ref().clone()).unwrap_or_default();
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(serve_wifi_setup_page(&saved_ssid, "").as_ref())
                .await?;
            return Ok(());
        }
        if ssid.is_none() {
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(serve_wifi_setup_page("", "SSID is not set").as_ref())
                .await?;
            return Ok(());
        }
        if password.is_none() {
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(serve_wifi_setup_page("", "SSID is not set").as_ref())
                .await?;
            return Ok(());
        }
        match wifi::save_wifi_creds(
            ssid.unwrap(),
            password.unwrap(),
            self.nvs.clone().as_ref().clone(),
        ) {
            Ok(_) => {
                conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                    .await?;
                conn.write_all(
                    serve_wifi_setup_page(
                        ssid.unwrap_or(&String::new()),
                        "Saved successfully, resetting now",
                    )
                    .as_ref(),
                )
                .await?;
                embassy_time::Timer::after_millis(50).await;
                reset::restart();
            }
            Err(e) => {
                conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                    .await?;
                conn.write_all(
                    serve_wifi_setup_page(
                        ssid.unwrap_or(&String::new()),
                        "COULD NOT SAVE WIFI CREDENTIALS, resetting now",
                    )
                    .as_ref(),
                )
                .await?;
                error!("{:?}", e);
                embassy_time::Timer::after_millis(50).await;
                reset::restart();
            }
        };
    }
    /*
       pub async fn spoolman_get_filaments<T, const N: usize>(
           &self,
           conn: &mut Connection<'_, T, N>,
       ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
       where
           T: Read + Write,
       {
           let spoolman_url = read_spoolman_url(self.nvs.as_ref().clone());
           if spoolman_url.is_none() {
               conn.initiate_response(400, None, &[("Content-Type", "application/json")])
                   .await?;
               conn.write_all(r#"{"status": "spoolman_url_not_set", "filaments": []}"#.as_ref())
                   .await?;
               return Ok(());
           }
           let mut client = Client::wrap(EspHttpConnection::new(&Default::default()).unwrap());
           let url = format!("{}/api/v1/filament", spoolman_url.unwrap());
           let req = client
               .request(Method::Get, &url, &[("accept", "application/json")])
               .unwrap();
           let res = req.submit();
           if res.is_err() {
               conn.initiate_response(500, None, &[("Content-Type", "application/json")])
                   .await?;
               conn.write_all(r#"{"status": "request_to_spoolman_failed", "filaments": []}"#.as_ref())
                   .await?;
               return Ok(());
           }
           let mut res = res.unwrap();
           let mut buf = [0u8; 4048];
           let _ = res.read(&mut buf);
           info!("Response: {}", String::from_utf8_lossy(&buf));
           let base_value: Value = serde_json::from_slice::<Value>(&buf).unwrap();
           let stream = base_value.as_array().unwrap();
           conn.initiate_response(200, None, &[("Content-Type", "application/json")])
               .await?;
           conn.write_all(r#"{"status": "request_to_spoolman_failed", "filaments": ["#.as_ref())
               .await?;
           for (i, value) in stream.iter().enumerate() {
               let mut data = format!(
                   r#"{{"name": "{}", "id": {}}}"#,
                   value.get("name").unwrap().as_str().unwrap(),
                   value.get("id").unwrap().as_i64().unwrap()
               );
               if i != 0 {
                   data = ",".to_string() + &data
               }
               conn.write_all(data.as_ref()).await?;
           }
           conn.write_all("]}".as_ref()).await?;
           return Ok(());
       }
    */
    pub async fn spoolman_set_filament<T, const N: usize>(
        &self,
        path: &str,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        if &*self.wifi_status.lock().unwrap() != &WifiEnum::Connected {
            conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Not connected to station, Spoolman unavailable."#.as_ref())
                .await?;
            return Ok(());
        }
        let url = Url::parse(&format!("http://google.com{}", path)).unwrap();
        let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let value = url_params.get("value");
        let filament_id = url_params.get("filament_id");
        if filament_id.is_none() || value.is_none() {
            conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Filament ID and/or Value are unset."#.as_ref())
                .await?;
            return Ok(());
        }
        let value: f32 = match value.unwrap().parse::<f32>() {
            Ok(d) => d,
            Err(_) => {
                conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                    .await?;
                conn.write_all(r#"Value is not an integer."#.as_ref())
                    .await?;
                return Ok(());
            }
        };
        let filament_id: i32 = match filament_id.unwrap().parse::<i32>() {
            Ok(d) => d,
            Err(_) => {
                conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                    .await?;
                conn.write_all(r#"Filament ID is not an integer."#.as_ref())
                    .await?;
                return Ok(());
            }
        };
        let spoolman_data = read_spoolman_data(self.nvs.as_ref().clone());
        if spoolman_data.0.is_none() || spoolman_data.0.clone().unwrap().is_empty() {
            conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Could not read storage."#.as_ref())
                .await?;
            return Ok(());
        }

        let mut client = Client::wrap(EspHttpConnection::new(&Default::default()).unwrap());
        let url = format!(
            "{}/api/v1/filament/{}",
            spoolman_data.0.unwrap(),
            filament_id
        );
        let payload = format!(
            r#"{{"extra": {{"{}": "{}"}}}}"#,
            spoolman_data.1.unwrap_or("td".to_string()),
            value
        );
        let payload_length = format!("{}", payload.len());
        let headers = [
            ("accept", "application/json"),
            ("content-type", "application/json"),
            ("content-length", &payload_length),
        ];
        let mut req = client.request(Method::Patch, &url, &headers).unwrap();
        req.write_all(payload.as_ref()).unwrap();
        req.flush().unwrap();
        let res = req.submit();
        if res.is_err() {
            conn.initiate_response(500, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Request to Spoolman failed!"#.as_ref())
                .await?;
            return Ok(());
        }
        let res = res.unwrap();
        if res.status() != 200 {
            conn.initiate_response(500, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Spoolman did not reply with 200"#.as_ref())
                .await?;
            return Ok(());
        }
        conn.initiate_response(302, None, &[("Location", "/")])
            .await?;

        Ok(())
    }
}

impl WsHandler<'_> {
    pub async fn algorithm_route<T, const N: usize>(
        &self,
        path: &str,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        let url = Url::parse(&format!("http://google.com{}", path)).unwrap();
        let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let m_value = url_params.get("m");
        let b_value = url_params.get("b");
        let threshold_value = url_params.get("threshold");
        let spoolman_value = url_params.get("spoolman_url");
        let spoolman_field_name = url_params.get("spoolman_field_name");
        if m_value.is_none()
            && b_value.is_none()
            && threshold_value.is_none()
            && spoolman_value.is_none()
        {
            let saved_algorithm = helpers::get_saved_algorithm_variables(self.nvs.as_ref().clone());
            let saved_spoolman = helpers::read_spoolman_data(self.nvs.as_ref().clone());
            let spoolman_url = match saved_spoolman.0 {
                Some(d) => d,
                None => "".to_string(),
            };
            let spoolman_field_name = match saved_spoolman.1 {
                Some(d) => d,
                None => "td".to_string(),
            };
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(
                serve_algo_setup_page(
                    saved_algorithm.b,
                    saved_algorithm.m,
                    saved_algorithm.threshold,
                    &spoolman_url,
                    &spoolman_field_name,
                )
                .as_ref(),
            )
            .await?;
            return Ok(());
        }
        let mod_b_value = b_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("0.0".to_string()));
        let mod_m_value = m_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("1.0".to_string()));
        let mod_threshold_value = threshold_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("0.5".to_string()));
        let mod_spoolman_value = spoolman_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("".to_string()));
        let mod_spoolman_field_name = spoolman_field_name
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("".to_string()));
        let save_spoolman_res = helpers::save_spoolman_data(
            &mod_spoolman_value,
            &mod_spoolman_field_name,
            self.nvs.as_ref().clone(),
        );
        if save_spoolman_res.is_err() {
            error!("{:?}", save_spoolman_res.err().unwrap());
            embassy_time::Timer::after_millis(50).await;
            reset::restart();
        }
        match helpers::save_algorithm_variables(
            &mod_b_value,
            &mod_m_value,
            &mod_threshold_value,
            self.nvs.as_ref().clone(),
        ) {
            Ok(_) => {
                conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                    .await?;
                conn.write_all(
                    serve_algo_setup_page(
                        mod_b_value.parse::<f32>().unwrap_or(0.0),
                        mod_m_value.parse::<f32>().unwrap_or(1.0),
                        mod_threshold_value.parse::<f32>().unwrap_or(0.5),
                        &mod_spoolman_value,
                        &mod_spoolman_field_name
                    )
                    .as_ref(),
                )
                .await?;
                #[allow(clippy::needless_return)]
                return Ok(());
            }
            Err(e) => {
                error!("{:?}", e);
                embassy_time::Timer::after_millis(50).await;
                reset::restart();
            }
        };
        // Ok(())
    }
}

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
            r#"{{"red": {:.2}, "green": {:.2}, "blue": {:.2}, "brightness": {:.2}, "td_reference": {:.2}, "reference_r": {}, "reference_g": {}, "reference_b": {}}}"#,
            multipliers.red, multipliers.green, multipliers.blue, multipliers.brightness, multipliers.td_reference,
            multipliers.reference_r, multipliers.reference_g, multipliers.reference_b
        );

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
        let content_length = conn.headers()?
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .unwrap_or(0);

        if content_length > buffer.len() {
            conn.initiate_response(400, Some("Payload too large"), &[]).await?;
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
                conn.initiate_response(400, Some("Invalid UTF-8"), &[]).await?;
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
        red = red.max(0.1).min(5.0);
        green = green.max(0.1).min(5.0);
        blue = blue.max(0.1).min(5.0);
        brightness = brightness.max(0.1).min(5.0);

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
        match helpers::save_rgb_multipliers(new_multipliers, self.nvs.as_ref().clone()) {
            Ok(_) => {
                conn.initiate_response(200, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(br#"{"status": "saved"}"#).await?;
            },
            Err(e) => {
                log::error!("Failed to save RGB multipliers: {:?}", e);
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
        log::info!("Starting optimization-based color calibration...");

        // Read the request body to get reference color
        let mut buffer = [0u8; 256];
        let mut total_read = 0;

        let content_length = conn.headers()?
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
            (multipliers.reference_r, multipliers.reference_g, multipliers.reference_b)
        };

        log::info!("Calibrating to target color: RGB({}, {}, {})", target_r, target_g, target_b);

        // Get the current median lux for this material using the buffer median
        let current_lux = {
            let buffer = self.lux_buffer.lock().unwrap();
            let median = buffer.median();
            if let Some(current_reading) = median {
                current_reading
            } else {
                conn.initiate_response(400, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(br#"{"status": "error", "message": "No valid lux buffer for normalization"}"#).await?;
                return Ok(());
            }
        };

        // Take current RAW median reading from the buffers
        let (current_r, current_g, current_b) = {
            let buffers = self.rgb_buffers.lock().unwrap();
            (
                buffers.0.median().unwrap_or(0),
                buffers.1.median().unwrap_or(0),
                buffers.2.median().unwrap_or(0),
            )
        };



        if current_r == 0 && current_g == 0 && current_b == 0 {
            conn.initiate_response(400, None, &[("Content-Type", "application/json")])
                .await?;
            conn.write_all(br#"{"status": "error", "message": "No valid color readings available"}"#).await?;
            return Ok(());
        }

        log::info!("Current raw median readings: R={}, G={}, B={}", current_r, current_g, current_b);

        // Get current multipliers as starting point for optimization
        let mut current_multipliers = {
            let multipliers = self.saved_rgb_multipliers.lock().unwrap();
            *multipliers
        };

        //set the current multiplier td to lux. TODO: Rename this field
        current_multipliers.td_reference = current_lux;

        log::info!("Starting optimization from current multipliers: R={:.3}, G={:.3}, B={:.3}, Brightness={:.3}",
                  current_multipliers.red, current_multipliers.green, current_multipliers.blue, current_multipliers.brightness);

        // Step 1: Optimize brightness to minimize overall RGB distance
        let optimized_brightness = optimize_brightness(
            (current_r, current_g, current_b),
            (target_r, target_g, target_b),
            self.rgb_baseline,
            current_lux,
            current_multipliers,
            100
        );

        current_multipliers.brightness = optimized_brightness;

        // Step 2: Fine-tune individual RGB channels
        let (optimized_red, optimized_green, optimized_blue) = optimize_rgb_channels(
            (current_r, current_g, current_b),
            (target_r, target_g, target_b),
            self.rgb_baseline,
            current_lux,
            current_multipliers,
            100
        );

        current_multipliers.red = optimized_red;
        current_multipliers.green = optimized_green;
        current_multipliers.blue = optimized_blue;

        log::info!("Optimization result: R={:.3}, G={:.3}, B={:.3}, Brightness={:.3}",
                  optimized_red, optimized_green, optimized_blue, optimized_brightness);

        // Verify the optimization result
        let verify_result = apply_complete_color_correction(
            (current_r, current_g, current_b).0,
            (current_r, current_g, current_b).1,
            (current_r, current_g, current_b).2,
            self.rgb_baseline,
            current_lux,
            &current_multipliers
        );

        log::info!("Verification - Target: RGB({},{},{}), Optimized result: RGB({},{},{})",
                  target_r, target_g, target_b, verify_result.0, verify_result.1, verify_result.2);

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

        log::info!("Setting new TD reference: {:.2} (was {:.2})", 
                  current_lux, current_multipliers.td_reference);

        // Update the in-memory multipliers
        {
            let mut multipliers = self.saved_rgb_multipliers.lock().unwrap();
            *multipliers = new_multipliers;
        }

        // Save to NVS
        match helpers::save_rgb_multipliers(new_multipliers, self.nvs.as_ref().clone()) {
            Ok(_) => {
                let response = format!(
                    r#"{{"status": "success", "red": {:.2}, "green": {:.2}, "blue": {:.2}, "brightness": {:.2}, "td_reference": {:.2}}}"#,
                    optimized_red, optimized_green, optimized_blue, optimized_brightness, current_lux
                );
                conn.initiate_response(200, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(response.as_bytes()).await?;
            },
            Err(e) => {
                log::error!("Failed to save optimized multipliers: {:?}", e);
                conn.initiate_response(500, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(br#"{"status": "error", "message": "Failed to save calibration"}"#).await?;
            }
        }

        Ok(())
    }
}

impl WsHandler<'_> {
    pub async fn fallback_route<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        // Try to acquire the BUSY lock without blocking
        let lock = BUSY.try_lock();
        let data = if let Ok(_guard) = lock {
            // We got the lock, run the function and update LAST_DATA
            let result = read_data_with_buffer(
                self.veml.clone(),
                self.veml_rgb.clone(),
                self.dark_baseline_reading,
                self.baseline_reading,
                self.rgb_baseline,
                self.dark_rgb_baseline,
                self.wifi_status.clone(),
                self.led_light.clone(),
                self.ws2812b.clone(),
                self.saved_algorithm,
                self.lux_buffer.clone(),
                self.rgb_buffers.clone(),
                self.saved_rgb_multipliers.clone(),
            )
            .await
            .unwrap_or_default();
            {
                let mut last = LAST_DATA.lock().unwrap();
                *last = Some(result.clone());
            }
            result
        } else {
            // Already running, serve the last result if available
            let last = LAST_DATA.lock().unwrap();
            last.clone().unwrap_or_else(|| "".to_string())
        };

        conn.initiate_response(200, None, &[("Content-Type", "text/raw")])
            .await?;
        conn.write_all(data.as_ref()).await?;
        Ok(())
    }
}

impl Handler for WsHandler<'_> {
    type Error<E>
        = WsHandlerError<EdgeError<E>>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Clone,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write,
    {
        let headers: &edge_http::RequestHeaders<'_, N> = conn.headers()?;

        if headers.method != EdgeMethod::Get && headers.method != EdgeMethod::Post {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path == "/" || headers.path.is_empty() {
            WsHandler::server_index_page(self, conn).await?;
        } else if headers.path.starts_with("/settings") {
            WsHandler::algorithm_route(self, headers.path, conn).await?;
        } else if headers.path.starts_with("/wifi") {
            WsHandler::wifi_route(self, headers.path, conn).await?;
        } else if headers.path.starts_with("/fallback") {
            WsHandler::fallback_route(self, conn).await?;
        } else if headers.path.starts_with("/spoolman/set") {
            WsHandler::spoolman_set_filament(self, headers.path, conn).await?;
        } else if headers.path == "/rgb_multipliers" {
            if headers.method == EdgeMethod::Get {
                WsHandler::get_rgb_multipliers(self, conn).await?;
            } else if headers.method == EdgeMethod::Post {
                WsHandler::set_rgb_multipliers(self, conn).await?;
            }
        } else if headers.path == "/auto_calibrate" && headers.method == EdgeMethod::Post {
            WsHandler::auto_calibrate_gray_reference(self, conn).await?;
        }
        /*else if headers.path.starts_with("/spoolman/filaments") {
            WsHandler::spoolman_get_filaments(self, conn).await?;
        } */
        else {
            conn.initiate_response(404, Some("Not found"), &[]).await?;
        }
        Ok(())
    }
}

fn apply_spectral_response_correction(r: u16, g: u16, b: u16, wb_r: u16, wb_g: u16, wb_b: u16) -> (u8, u8, u8) {
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

    log::info!("Spectral correction: Raw({},{},{}) -> WB factors({:.3},{:.3},{:.3}) -> Final({},{},{})",
               r, g, b, r_correction, g_correction, b_correction, r_final, g_final, b_final);

    (r_final, g_final, b_final)
}

// New helper function to apply complete color correction pipeline
fn apply_complete_color_correction(
    raw_r: u16,
    raw_g: u16,
    raw_b: u16,
    white_balance: (u16, u16, u16),
    current_lux: f32,
    multipliers: &RGBMultipliers
) -> (u8, u8, u8) {
    // Step 1: Apply spectral response correction
    let (corrected_r, corrected_g, corrected_b) = apply_spectral_response_correction(
        raw_r, raw_g, raw_b,
        white_balance.0, white_balance.1, white_balance.2
    );

    // Step 2: Apply RGB multipliers with lux-based brightness normalization
    apply_rgb_multipliers(corrected_r, corrected_g, corrected_b, current_lux, multipliers)
}



fn apply_rgb_multipliers(
    r: u8,
    g: u8,
    b: u8,
    current_lux: f32,
    multipliers: &RGBMultipliers
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

    let r_final = (r_color_corrected * total_brightness).round().clamp(0.0, 255.0) as u8;
    let g_final = (g_color_corrected * total_brightness).round().clamp(0.0, 255.0) as u8;
    let b_final = (b_color_corrected * total_brightness).round().clamp(0.0, 255.0) as u8;

    log::info!(
        "Lux-normalized correction: ({},{},{}) * Color({:.2},{:.2},{:.2}) * Brightness({:.2}) * Norm({:.2}) = ({},{},{})",
        r, g, b, multipliers.red, multipliers.green, multipliers.blue, multipliers.brightness, normalization_factor,
        r_final, g_final, b_final
    );

    (r_final, g_final, b_final)
}

// Helper function to calculate RGB distance
fn calculate_rgb_distance(color1: (u8, u8, u8), color2: (u8, u8, u8)) -> f32 {
    let dr = color1.0 as f32 - color2.0 as f32;
    let dg = color1.1 as f32 - color2.1 as f32;
    let db = color1.2 as f32 - color2.2 as f32;
    (dr * dr + dg * dg + db * db).sqrt()
}

// Optimize brightness to minimize overall RGB distance
fn optimize_brightness(
    raw_color: (u16, u16, u16),
    target_color: (u8, u8, u8),
    white_balance: (u16, u16, u16),
    current_lux: f32,
    mut multipliers: RGBMultipliers,
    max_iterations: usize
) -> f32 {
    let mut best_brightness = multipliers.brightness;
    let mut best_distance = f32::MAX;

    // Current distance
    let current_result = apply_complete_color_correction(
        raw_color.0, raw_color.1, raw_color.2,
        white_balance,
        current_lux,
        &multipliers
    );
    let mut current_distance = calculate_rgb_distance(current_result, target_color);
    best_distance = current_distance;

    log::info!("Brightness optimization start: brightness={:.3}, distance={:.2}",
              multipliers.brightness, current_distance);

    let step_size = 0.02; // 5% steps
    let mut step_direction = 0; // 0=unknown, 1=increase, -1=decrease

    for iteration in 0..max_iterations {
        let mut improved = false;

        // Try both directions if we don't know the direction yet
        let directions = if step_direction == 0 { vec![1.0, -1.0] } else { vec![step_direction as f32] };

        for &direction in &directions {
            let test_brightness = (multipliers.brightness + direction * step_size).clamp(0.1, 3.0);

            let mut test_multipliers = multipliers;
            test_multipliers.brightness = test_brightness;

            let test_result = apply_complete_color_correction(
                raw_color.0, raw_color.1, raw_color.2,
                white_balance,
                current_lux,
                &test_multipliers
            );

            let test_distance = calculate_rgb_distance(test_result, target_color);

            if test_distance < best_distance {
                best_distance = test_distance;
                best_brightness = test_brightness;
                step_direction = direction as i32;
                improved = true;

                log::debug!("Brightness iter {}: {:.3} -> distance {:.2} (improved)",
                          iteration, test_brightness, test_distance);
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

    log::info!("Brightness optimization complete: {:.3} -> {:.3}, distance: {:.2} -> {:.2}",
              multipliers.brightness, best_brightness, current_distance, best_distance);

    best_brightness
}

// Fine-tune individual RGB channels
fn optimize_rgb_channels(
    raw_color: (u16, u16, u16),
    target_color: (u8, u8, u8),
    white_balance: (u16, u16, u16),
    current_lux: f32,
    mut multipliers: RGBMultipliers,
    max_iterations: usize
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

        let mut best_channel_distance = f32::MAX;
        let mut step_direction = 0; // 0=unknown, 1=increase, -1=decrease

        // Get initial channel distance
        let initial_result = apply_complete_color_correction(
            raw_color.0, raw_color.1, raw_color.2,
            white_balance,
            current_lux,
            &multipliers
        );

        let initial_channel_value = match *channel {
            "red" => initial_result.0,
            "green" => initial_result.1,
            "blue" => initial_result.2,
            _ => 127,
        };

        best_channel_distance = (initial_channel_value as f32 - target_channel_value as f32).abs();

        log::info!("{} channel optimization start: multiplier={:.3}, current={}, target={}, distance={:.2}",
                  channel, best_value, initial_channel_value, target_channel_value, best_channel_distance);

        for iteration in 0..max_iterations {
            let mut improved = false;

            // Try both directions if we don't know the direction yet
            let directions = if step_direction == 0 { vec![1.0, -1.0] } else { vec![step_direction as f32] };

            for &direction in &directions {
                let test_value = (best_value + direction * step_size).clamp(0.5, 2.0);

                let mut test_multipliers = multipliers;
                match *channel {
                    "red" => test_multipliers.red = test_value,
                    "green" => test_multipliers.green = test_value,
                    "blue" => test_multipliers.blue = test_value,
                    _ => {},
                }

                let test_result = apply_complete_color_correction(
                    raw_color.0, raw_color.1, raw_color.2,
                    white_balance,
                    current_lux,
                    &test_multipliers
                );

                let test_channel_value = match *channel {
                    "red" => test_result.0,
                    "green" => test_result.1,
                    "blue" => test_result.2,
                    _ => 127,
                };

                let test_channel_distance = (test_channel_value as f32 - target_channel_value as f32).abs();

                if test_channel_distance < best_channel_distance {
                    best_channel_distance = test_channel_distance;
                    best_value = test_value;
                    step_direction = direction as i32;
                    improved = true;

                    log::debug!("{} iter {}: {:.3} -> value {} distance {:.2} (improved)",
                              channel, iteration, test_value, test_channel_value, test_channel_distance);
                    break;
                }
            }

            if improved {
                match *channel {
                    "red" => multipliers.red = best_value,
                    "green" => multipliers.green = best_value,
                    "blue" => multipliers.blue = best_value,
                    _ => {},
                }
            } else {
                // No improvement found for this channel, move to next
                break;
            }
        }

        log::info!("{} channel optimization complete: {:.3}, distance: {:.2}",
                  channel, best_value, best_channel_distance);
    }

    (multipliers.red, multipliers.green, multipliers.blue)
}

async fn read_data_with_buffer(
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
    rgb_buffers: Arc<Mutex<(RunningMedianBufferU16, RunningMedianBufferU16, RunningMedianBufferU16)>>,
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
    log::info!("Current LED brightness: {:?}", current_led_brightness);

    // Only lock again if needed, and drop immediately after
    if current_led_brightness != 25 {
        log::info!("Setting LED to fully on for filament detection");
        {
            let mut led = led_light.lock().unwrap();
            if let Err(e) = led.set_duty(25) {
                log::error!("Failed to set LED duty cycle: {:?}", e);
                return None;
            }
        }
        embassy_time::Timer::after_millis(350).await;
    }

    for i in 0..3 {
        let current_reading = {
            let mut locked_veml = veml.lock().unwrap();
            match locked_veml.read_lux() {
                Ok(d) => d as f32,
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
    let variance = detection_readings.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / 3.0;
    let std_dev = variance.sqrt();

    log::info!("Filament detection readings: [{:.2}, {:.2}, {:.2}] -> median: {:.2}, std_dev: {:.3}",
              detection_readings[0], detection_readings[1], detection_readings[2], median_reading, std_dev);
    
    // Warn if readings are too similar (might indicate sensor issue)
    if std_dev < 0.1 && median_reading > 10.0 {
        log::warn!("VEML7700 readings very similar (std_dev: {:.3}) - sensor might need more time", std_dev);
    }

    let brightness_diff = dark_baseline_reading;
    let current_threshold = dark_baseline_reading - (1.0 - saved_algorithm.threshold) * brightness_diff;
    log::info!("Detection threshold check: {:.2} (threshold: {:.2})",
              median_reading, current_threshold);

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
            log::error!("Failed to set LED duty cycle: {:?}", e);
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
            let lux_reading = locked_veml.read_lux().unwrap_or(0.0) as f32;
            buffer.push(lux_reading);
        }

        let mut locked_rgb = veml_rgb.lock().unwrap();
        if let (Ok(r), Ok(g), Ok(b)) = (locked_rgb.read_red(), locked_rgb.read_green(), locked_rgb.read_blue()) {
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

    log::info!("Final TD value: {:.2} (raw lux: {:.2}, baseline: {:.2}, m: {:.3}, b: {:.3})",
               adjusted_td_value, final_median_lux, baseline_reading, saved_algorithm.m, saved_algorithm.b);

    // Read clear channel for brightness correction (RAW)
    let clear_median_raw = {
        let mut locked_rgb = veml_rgb.lock().unwrap();
        locked_rgb.read_clear().unwrap_or(rgb_white_balance.0)
    };

    log::debug!("RAW median values: Lux={:.2}, RGB=({},{},{}), Clear={}",
               final_median_lux, r_median_raw, g_median_raw, b_median_raw, clear_median_raw);

    // NOW apply calibration/correction to the RAW median values
    // Step 1: Apply spectral response correction to RAW medians
    let (r_corrected, g_corrected, b_corrected) = apply_spectral_response_correction(
        r_median_raw, g_median_raw, b_median_raw,
        rgb_white_balance.0, rgb_white_balance.1, rgb_white_balance.2
    );

    log::info!("Spectral corrected RGB: ({},{},{})", r_corrected, g_corrected, b_corrected);

    // Step 2: Apply user RGB multipliers with lux-based brightness adjustment to corrected values
    let (r_final, g_final, b_final) = {
        let multipliers = rgb_multipliers.lock().unwrap();
        apply_rgb_multipliers(r_corrected, g_corrected, b_corrected, final_median_lux, &*multipliers)
    };

    // Create hex color string with corrected values
    let hex_color = format!("#{:02X}{:02X}{:02X}", r_final, g_final, b_final);

    let ws_message = format!("{:.2},{},{}", adjusted_td_value, hex_color, buffer_count);

    // Log buffer status and detailed color information
    let (lux_len, rgb_len) = {
        let lux_buf = lux_buffer.lock().unwrap();
        let rgb_buf = rgb_buffers.lock().unwrap();
        (lux_buf.len(), rgb_buf.0.len())
    };

    log::info!("Reading: {:.2}, RGB: {} (medians from {} lux, {} RGB samples, confidence: {}), Raw RGB: ({},{},{}), Final RGB: ({},{},{}) - Baseline: {:.2}, Lux: {}, Clear: {}",
               adjusted_td_value, hex_color, lux_len, rgb_len, buffer_count,
               r_median_raw, g_median_raw, b_median_raw,
               r_final, g_final, b_final,
               saved_algorithm.b, final_median_lux, clear_median_raw);

    Some(ws_message)
}

// Static for concurrency control and caching last result
static BUSY: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static LAST_DATA: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
