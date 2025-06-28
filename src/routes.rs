use std::{
    borrow::Cow,
    collections::HashMap,
    str,
    sync::{Arc, Mutex},
    fmt::{Debug, Display},
};

use edge_http::io::server::{Connection, Handler};
use edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
use edge_http::Method as EdgeMethod;
use edge_ws::{FrameHeader, FrameType};
use embedded_hal::pwm::SetDutyCycle;
use embedded_io_async::{Read, Write};
use embedded_svc::http::client::Client;
use esp_idf_svc::{
    hal::{ledc::LedcDriver, reset},
    http::{client::EspHttpConnection, Method},
    io::Write as _,
};
use log::error;
use url::Url;
use veml7700::Veml7700;

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

// #[derive(Deserialize, Debug)]
// struct SpoolmanFilamentResponse {
//     id: u32,
//     name: String,
// }
impl WsHandler<'_> {
    pub async fn server_index_page<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
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
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
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
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
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
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
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
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
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
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
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
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
    where
        T: Read + Write,
    {
        log::info!("Starting automatic reference color calibration...");

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
            // Use current saved reference color
            let multipliers = self.saved_rgb_multipliers.lock().unwrap();
            (multipliers.reference_r, multipliers.reference_g, multipliers.reference_b)
        };

        log::info!("Calibrating to reference color: RGB({}, {}, {})", target_r, target_g, target_b);

        // Get the current TD value for this material
        let current_td = {
            let mut locked_veml = self.veml.lock().unwrap();
            match locked_veml.read_lux() {
                Ok(reading) => {
                    let current_reading = reading as f32;
                    let td_value = (current_reading / self.baseline_reading) * 100.0;
                    self.saved_algorithm.m * td_value + self.saved_algorithm.b
                },
                Err(_) => {
                    conn.initiate_response(400, None, &[("Content-Type", "application/json")])
                        .await?;
                    conn.write_all(br#"{"status": "error", "message": "Cannot read sensor for TD calculation"}"#).await?;
                    return Ok(());
                }
            }
        };

        log::info!("Current TD value for calibration: {:.2}", current_td);

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

        // Read clear channel
        let clear_median = {
            let mut locked_rgb = self.veml_rgb.lock().unwrap();
            locked_rgb.read_clear().unwrap_or(current_r)
        };

        // Apply ONLY the base color correction (white balance and spectral response)
        // WITHOUT any user multipliers to get the "natural" corrected color
        let wb_clear_estimate = (self.rgb_baseline.0 + self.rgb_baseline.1 + self.rgb_baseline.2) as f32 * 1.2;
        let (current_corrected_r, current_corrected_g, current_corrected_b) = apply_advanced_color_correction(
            current_r, current_g, current_b, clear_median,
            self.rgb_baseline.0, self.rgb_baseline.1, self.rgb_baseline.2, wb_clear_estimate as u16
        );

        log::info!("Current corrected color (no user multipliers): R={}, G={}, B={}", 
                  current_corrected_r, current_corrected_g, current_corrected_b);

        // Get the PRESERVED TD reference value - NEVER change this during calibration
        let preserved_td_reference = {
            let multipliers = self.saved_rgb_multipliers.lock().unwrap();
            multipliers.td_reference
        };

        // Calculate TD-based brightness compensation factor using the PRESERVED reference
        // If current TD is lower than reference, the material is more opaque and needs brightness boost
        // If current TD is higher than reference, the material is more transparent and needs brightness reduction
        let td_brightness_compensation = if preserved_td_reference > 0.1 {
            preserved_td_reference / current_td.max(0.1) // Avoid division by zero
        } else {
            1.0 // No compensation if no reference TD
        };

        log::info!("TD brightness compensation factor: {:.3} (preserved_ref_td={:.2}, current_td={:.2})",
                  td_brightness_compensation, preserved_td_reference, current_td);

        // Apply TD compensation to target color to get the expected corrected color for this material
        let td_compensated_target_r = (target_r as f32 / td_brightness_compensation).max(1.0).min(255.0);
        let td_compensated_target_g = (target_g as f32 / td_brightness_compensation).max(1.0).min(255.0);
        let td_compensated_target_b = (target_b as f32 / td_brightness_compensation).max(1.0).min(255.0);

        log::info!("TD compensated target color: R={:.1}, G={:.1}, B={:.1}",
                  td_compensated_target_r, td_compensated_target_g, td_compensated_target_b);

        // Calculate what RGB multipliers are needed to transform current corrected color to TD-compensated target
        let red_multiplier = if current_corrected_r > 0 {
            td_compensated_target_r / current_corrected_r as f32
        } else {
            1.0 
        };
        
        let green_multiplier = if current_corrected_g > 0 { 
            td_compensated_target_g / current_corrected_g as f32
        } else {
            1.0 
        };
        
        let blue_multiplier = if current_corrected_b > 0 { 
            td_compensated_target_b / current_corrected_b as f32
        } else {
            1.0 
        };

        // Keep the current brightness multiplier unchanged
        let current_brightness = {
            let multipliers = self.saved_rgb_multipliers.lock().unwrap();
            multipliers.brightness
        };

        // Clamp multipliers to reasonable ranges
        let final_red = red_multiplier.max(0.1).min(5.0);
        let final_green = green_multiplier.max(0.1).min(5.0);
        let final_blue = blue_multiplier.max(0.1).min(5.0);

        log::info!("Calculated RGB multipliers: R={:.3}, G={:.3}, B={:.3}, Brightness={:.3} (unchanged)", 
                  final_red, final_green, final_blue, current_brightness);

        // Verify the calculation by simulating what the final color will be with TD compensation
        let verify_with_td = apply_td_based_brightness_correction(
            current_corrected_r, current_corrected_g, current_corrected_b,
            current_td,
            &RGBMultipliers {
                red: final_red,
                green: final_green,
                blue: final_blue,
                brightness: current_brightness,
                td_reference: preserved_td_reference, // Use preserved value
                reference_r: target_r,
                reference_g: target_g,
                reference_b: target_b,
            }
        );

        log::info!("Verification - Target: RGB({},{},{}), Calculated result: RGB({},{},{})",
                  target_r, target_g, target_b, verify_with_td.0, verify_with_td.1, verify_with_td.2);

        let new_multipliers = RGBMultipliers {
            red: final_red,
            green: final_green,
            blue: final_blue,
            brightness: current_brightness, // Keep current brightness unchanged
            td_reference: preserved_td_reference, // PRESERVE the original TD reference - NEVER change this
            reference_r: target_r,
            reference_g: target_g,
            reference_b: target_b,
        };

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
                    final_red, final_green, final_blue, current_brightness, preserved_td_reference
                );
                conn.initiate_response(200, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(response.as_bytes()).await?;
            },
            Err(e) => {
                log::error!("Failed to save auto-calibrated multipliers: {:?}", e);
                conn.initiate_response(500, None, &[("Content-Type", "application/json")])
                    .await?;
                conn.write_all(br#"{"status": "error", "message": "Failed to save calibration"}"#).await?;
            }
        }

        Ok(())
    }

    // Keep the old method name for backwards compatibility
    pub async fn auto_calibrate_white_reference<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
    where
        T: Read + Write,
    {
        self.auto_calibrate_gray_reference(conn).await
    }
}

impl WsHandler<'_> {
    pub async fn fallback_route<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
    where
        T: Read + Write,
    {
        let data = read_data_with_buffer(
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
        conn.initiate_response(200, None, &[("Content-Type", "text/raw")])
            .await?;
        conn.write_all(data.as_ref()).await?;
        Ok(())
    }

    pub async fn averaged_reading_route<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
    where
        T: Read + Write,
    {
        let data = read_averaged_data_with_buffer(
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
        conn.initiate_response(200, None, &[("Content-Type", "text/raw")])
            .await?;
        conn.write_all(data.as_ref()).await?;
        Ok(())
    }
}

impl Handler for WsHandler<'_> {
    type Error<E>
        = WsHandlerError<EdgeError<E>, edge_ws::Error<E>>
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
        } else if headers.path.starts_with("/averaged") {
            WsHandler::averaged_reading_route(self, conn).await?;
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
        else if headers.path.starts_with("/ws") {
            match WsHandler::ws_handler(self, conn).await {
                Ok(_) => (),
                Err(e) => {
                    log::error!("WS Error: {:?}", e);
                    return Err(e);
                }
            };
        } else {
            conn.initiate_response(404, Some("Not found"), &[]).await?;
        }
        Ok(())
    }
}

impl WsHandler<'_> {
    pub async fn ws_handler<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
    where
        T: Read + Write,
    {
        let mut buf = unsafe { Box::<[u8; 8192]>::new_uninit().assume_init() };
        let buf = buf.as_mut_slice();
        let resp_buf = &mut buf[..MAX_BASE64_KEY_RESPONSE_LEN];
        conn.initiate_ws_upgrade_response(resp_buf.try_into().unwrap())
            .await?;
        conn.complete().await?;
        let mut socket = conn.unbind()?;

        loop {
            // header.mask_key = None; // Servers never mask the payload

            // if matches!(header.frame_type, FrameType::Ping) {
            //     header.frame_type = FrameType::Pong;
            // }
            let mut recv_header = FrameHeader::recv(&mut socket)
                .await
                .map_err(WsHandlerError::Ws)?;
            let payload = recv_header
                .recv_payload(&mut socket, buf)
                .await
                .map_err(WsHandlerError::Ws)?;

            recv_header.mask_key = None; // Servers never mask the payload

            if matches!(recv_header.frame_type, FrameType::Ping) {
                recv_header.frame_type = FrameType::Pong;
            }
            recv_header
                .send(&mut socket)
                .await
                .map_err(WsHandlerError::Ws)?;
            recv_header
                .send_payload(&mut socket, payload)
                .await
                .map_err(WsHandlerError::Ws)?;

            let td_value = read_data_with_buffer(
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
            .await;
            let payload = match td_value {
                Some(d) => d,
                None => "error".to_string(),
            };
            log::info!("length: {:?}, data: {payload}", payload.len() as u64);
            let header = FrameHeader {
                frame_type: FrameType::Text(false),
                payload_len: payload.len() as u64,
                mask_key: None,
            };
            header.send(&mut socket).await.map_err(WsHandlerError::Ws)?;
            header
                .send_payload(&mut socket, payload.as_ref())
                .await
                .map_err(WsHandlerError::Ws)?;
            embassy_time::Timer::after_millis(125).await; // Increased rate by 4x (was 500ms)
        }
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

fn apply_advanced_color_correction(
    r: u16, g: u16, b: u16, clear: u16,
    wb_r: u16, wb_g: u16, wb_b: u16, wb_clear: u16
) -> (u8, u8, u8) {
    // Step 1: Apply spectral response correction
    let (r_spec, g_spec, b_spec) = apply_spectral_response_correction(r, g, b, wb_r, wb_g, wb_b);

    // Step 2: Apply brightness correction only if needed
    let avg_intensity = (r_spec as f32 + g_spec as f32 + b_spec as f32) / 3.0;

    // Only apply brightness boost for very dark samples
    if avg_intensity < 20.0 && clear > 0 && wb_clear > 0 {
        let transmission_ratio = (clear as f32 / wb_clear as f32).min(1.0).max(0.05);
        let brightness_boost = (1.0 / transmission_ratio.powf(0.5)).min(3.0); // Cap at 3x boost

        let r_boosted = (r_spec as f32 * brightness_boost).round().min(255.0) as u8;
        let g_boosted = (g_spec as f32 * brightness_boost).round().min(255.0) as u8;
        let b_boosted = (b_spec as f32 * brightness_boost).round().min(255.0) as u8;

        log::info!("Applied brightness boost {:.2}x for dark sample: ({},{},{}) -> ({},{},{})",
                   brightness_boost, r_spec, g_spec, b_spec, r_boosted, g_boosted, b_boosted);

        return (r_boosted, g_boosted, b_boosted);
    }

    log::info!("Using spectral corrected values: ({},{},{})", r_spec, g_spec, b_spec);
    (r_spec, g_spec, b_spec)
}

fn apply_td_based_brightness_correction(
    r: u8, g: u8, b: u8,
    current_td: f32,
    multipliers: &RGBMultipliers
) -> (u8, u8, u8) {
    // Calculate the TD-based brightness factor
    // Linear relationship: higher TD = more transmission = brighter base
    let td_ratio = multipliers.td_reference / current_td.max(0.1); // Avoid division by zero

    // Apply TD-based automatic brightness adjustment
    let auto_brightness_factor = td_ratio.max(0.1).min(10.0); // Clamp to reasonable range

    log::debug!("TD-based brightness: current_td={:.2}, ref_td={:.2}, ratio={:.3}, auto_factor={:.3}",
               current_td, multipliers.td_reference, td_ratio, auto_brightness_factor);

    // Apply color multipliers first
    let r_color_corrected = r as f32 * multipliers.red;
    let g_color_corrected = g as f32 * multipliers.green;
    let b_color_corrected = b as f32 * multipliers.blue;

    // Apply both manual brightness and automatic TD-based brightness
    let total_brightness = multipliers.brightness * auto_brightness_factor;

    let r_final = (r_color_corrected * total_brightness).round().min(255.0).max(0.0) as u8;
    let g_final = (g_color_corrected * total_brightness).round().min(255.0).max(0.0) as u8;
    let b_final = (b_color_corrected * total_brightness).round().min(255.0).max(0.0) as u8;

    log::debug!("TD-brightness correction: ({},{},{}) * Color({:.2},{:.2},{:.2}) * Total_Brightness({:.2}) = ({},{},{})",
               r, g, b, multipliers.red, multipliers.green, multipliers.blue, total_brightness,
               r_final, g_final, b_final);

    (r_final, g_final, b_final)
}

fn apply_rgb_multipliers(r: u8, g: u8, b: u8, current_td: f32, multipliers: &RGBMultipliers) -> (u8, u8, u8) {
    apply_td_based_brightness_correction(r, g, b, current_td, multipliers)
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

    // {
    //     let mut led = led_light.lock().unwrap();
    //     led.set_duty_cycle_fully_on().unwrap()
    // }
    // embassy_time::Timer::after_millis(15).await;

    // Take a quick reading first for fast filament detection
    let current_reading = {
        let mut locked_veml = veml.lock().unwrap();
        match locked_veml.read_lux() {
            Ok(d) => d as f32,
            Err(e) => {
                log::error!("Failed to read sensor: {:?}", e);
                return None;
            }
        }
    };
    log::info!("Measured lux: {:.2}", current_reading);
    //print saved_algorithm.threshold
    log::info!("Saved algorithm threshold: {:.2}", saved_algorithm.threshold);
    //print current_reading / dark_baseline_reading
    log::info!("Current reading / dark baseline: {:.2}", current_reading / dark_baseline_reading);

    // Use current reading for fast "no filament" detection
    if current_reading / dark_baseline_reading > saved_algorithm.threshold {
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

    // Filament is detected, now do proper measurement with median filtering
    log::info!("Filament detected!");
    set_led(ws2812.clone(), 0, 125, 125);
    {
        let mut led = led_light.lock().unwrap();
        if let Err(e) = led.set_duty_cycle_fully_on() {
            log::error!("Failed to set LED duty cycle: {:?}", e);
            return None;
        }
    }

    // Take multiple readings for median calculation - keep consistent count regardless of buffer size
    let readings_per_call = 3;
    for _ in 0..readings_per_call {
        embassy_time::Timer::after_millis(15).await;
        let mut locked_veml = veml.lock().unwrap();
        if let Ok(clr) = locked_veml.read_lux() {
            let mut buffer = lux_buffer.lock().unwrap();
            buffer.push(clr as f32);
        }

        let mut locked_rgb = veml_rgb.lock().unwrap();
        if let (Ok(r), Ok(g), Ok(b)) = (locked_rgb.read_red(), locked_rgb.read_green(), locked_rgb.read_blue()) {
            log::debug!("RGB readings: R={}, G={}, B={}", r, g, b);

            let mut buffers = rgb_buffers.lock().unwrap();
            buffers.0.push(r);
            buffers.1.push(g);
            buffers.2.push(b);
        }
    }

    // Get buffer count for confidence indicator
    let buffer_count = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.len()
    };

    // Get median values for accurate measurement
    let final_median_lux = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.median().unwrap_or(current_reading)
    };

    let (r_median_raw, g_median_raw, b_median_raw) = {
        let buffers = rgb_buffers.lock().unwrap();
        (
            buffers.0.median().unwrap_or(rgb_white_balance.0),
            buffers.1.median().unwrap_or(rgb_white_balance.1),
            buffers.2.median().unwrap_or(rgb_white_balance.2),
        )
    };

    // Calculate TD from RAW lux reading - FIXED CALCULATION
    let td_value = (final_median_lux / baseline_reading) * 100.0;
    let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;

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

    // Step 2: Apply user RGB multipliers with TD-based brightness adjustment to corrected values
    let (r_final, g_final, b_final) = {
        let multipliers = rgb_multipliers.lock().unwrap();
        apply_rgb_multipliers(r_corrected, g_corrected, b_corrected, adjusted_td_value, &*multipliers)
    };

    // Create hex color string with corrected values
    let hex_color = format!("#{:02X}{:02X}{:02X}", r_final, g_final, b_final);

    let ws_message = format!("{:.2},{},{}", adjusted_td_value, hex_color, buffer_count);

    {
        let mut led = led_light.lock().unwrap();
        if let Err(e) = led.set_duty(25) {
            log::error!("Failed to adjust LED duty: {:?}", e);
        }
    }

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

pub async fn read_averaged_data_with_buffer(
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
    // Take a quick reading first for fast filament detection
    let mut locked_veml = veml.lock().unwrap();
    let current_reading = match locked_veml.read_lux() {
        Ok(d) => d as f32,
        Err(e) => {
            log::error!("Failed to read sensor: {:?}", e);
            return None;
        }
    };

    // Use current reading for fast "no filament" detection
    if current_reading / dark_baseline_reading > saved_algorithm.threshold {
        let wifi_stat = wifi_status.lock().unwrap();
        match *wifi_stat {
            WifiEnum::Connected => set_led(ws2812.clone(), 0, 255, 0),
            WifiEnum::HotSpot => set_led(ws2812.clone(), 255, 0, 255),
            WifiEnum::Working => set_led(ws2812.clone(), 255, 255, 0),
        }
        log::info!("No filament detected!");
        return Some("no_filament".to_string());
    }

    // Filament is detected, proceed with intensive sampling
    log::info!("Filament detected!");
    set_led(ws2812.clone(), 0, 125, 125);
    {
        let mut led = led_light.lock().unwrap();
        if let Err(e) = led.set_duty_cycle_fully_on() {
            log::error!("Failed to set LED duty cycle: {:?}", e);
            return None;
        }
    }

    // Clear buffers for fresh sampling
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

    embassy_time::Timer::after_millis(10).await;

    // For averaged data, we'll take more intensive sampling - store RAW values only
    let sample_count = 100;
    let mut clear_readings_raw = Vec::with_capacity(sample_count);

    // Take many rapid samples to fill the median buffer with RAW values
    for _ in 0..sample_count {
        let clr = match locked_veml.read_lux() {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to read sensor: {:?}", e);
                continue;
            }
        };

        // Store RAW lux value
        {
            let mut buffer = lux_buffer.lock().unwrap();
            buffer.push(clr as f32);
        }

        // Store RAW RGB values
        let mut locked_rgb = veml_rgb.lock().unwrap();
        if let (Ok(r), Ok(g), Ok(b), Ok(clear)) = (
            locked_rgb.read_red(),
            locked_rgb.read_green(),
            locked_rgb.read_blue(),
            locked_rgb.read_clear()
        ) {
            let mut buffers = rgb_buffers.lock().unwrap();
            buffers.0.push(r); // Store RAW red
            buffers.1.push(g); // Store RAW green
            buffers.2.push(b); // Store RAW blue
            clear_readings_raw.push(clear); // Store RAW clear
        }

        embassy_time::Timer::after_millis(12).await;
    }

    {
        let mut led = led_light.lock().unwrap();
        if let Err(e) = led.set_duty(25) {
            log::error!("Failed to adjust LED duty: {:?}", e);
        }
    }

    // Get buffer count for confidence indicator
    let buffer_count = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.len()
    };

    // Get RAW median values from the filled buffers
    let median_lux_raw = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.median().unwrap_or(current_reading)
    };

    let (r_median_raw, g_median_raw, b_median_raw) = {
        let buffers = rgb_buffers.lock().unwrap();
        (
            buffers.0.median().unwrap_or(rgb_white_balance.0),
            buffers.1.median().unwrap_or(rgb_white_balance.1),
            buffers.2.median().unwrap_or(rgb_white_balance.2),
        )
    };

    // Calculate RAW clear median
    let clear_median_raw = if !clear_readings_raw.is_empty() {
        clear_readings_raw.sort();
        clear_readings_raw[clear_readings_raw.len() / 2]
    } else {
        rgb_white_balance.0 // Fallback
    };

    // Calculate TD from RAW lux reading - FIXED CALCULATION
    let td_value = (median_lux_raw / baseline_reading) * 100.0;
    let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;

    log::debug!("RAW median values: Lux={:.2}, RGB=({},{},{}), Clear={}",
               median_lux_raw, r_median_raw, g_median_raw, b_median_raw, clear_median_raw);

    // NOW apply calibration/correction to the RAW median values
    // Step 1: Apply spectral response correction to RAW medians
    let (r_corrected, g_corrected, b_corrected) = apply_spectral_response_correction(
        r_median_raw, g_median_raw, b_median_raw,
        rgb_white_balance.0, rgb_white_balance.1, rgb_white_balance.2
    );

    log::info!("Spectral corrected RGB: ({},{},{})", r_corrected, g_corrected, b_corrected);

    // Step 2: Apply user RGB multipliers with TD-based brightness adjustment to corrected values
    let (r_final, g_final, b_final) = {
        let multipliers = rgb_multipliers.lock().unwrap();
        apply_rgb_multipliers(r_corrected, g_corrected, b_corrected, adjusted_td_value, &*multipliers)
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

    log::info!("Reading: {:.2}, RGB: {} (medians from {} lux, {} RGB samples, confidence: {}), Raw RGB: ({},{},{}), Final RGB: ({},{},{})",
              adjusted_td_value, hex_color, lux_len, rgb_len, buffer_count, r_median_raw, g_median_raw, b_median_raw, r_final, g_final, b_final);

    Some(ws_message)
}

