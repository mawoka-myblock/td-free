use std::{
    borrow::Cow,
    collections::HashMap,
    str,
    sync::{Arc, Mutex},
};

use edge_http::io::server::Connection;
use edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
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
    helpers::{self, read_spoolman_data, NvsData, HardwareI2cInstance, SimpleBitBangI2cInstance},
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
            .unwrap_or_else(|| Cow::Owned("0.8".to_string()));
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
                        mod_threshold_value.parse::<f32>().unwrap_or(0.8),
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
        )
        .await
        .unwrap_or_default();
        conn.initiate_response(200, None, &[("Content-Type", "text/raw")])
            .await?;
        conn.write_all(data.as_ref()).await?;
        Ok(())
    }
}

impl WsHandler<'_> {
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
        )
        .await
        .unwrap_or_default();
        conn.initiate_response(200, None, &[("Content-Type", "text/raw")])
            .await?;
        conn.write_all(data.as_ref()).await?;
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

fn normalize_rgb_value(value: u16, baseline: u16) -> u8 {
    if baseline == 0 {
        return 0;
    }
    
    // Scale to 0-255 range, with some amplification for visibility
    let scaled = ((value as f32 / baseline as f32) * 180.0).round() as u16;
    (scaled.min(255)) as u8
}

fn normalize_rgb_value_with_white_balance(value: u16, white_balance: u16) -> u8 {
    if white_balance == 0 {
        return 0;
    }

    // White balance correction: normalize against the white reference
    // Scale to 0-255 range based on the proportion of the measured value to white balance
    let normalized = (value as f32 / white_balance as f32) * 255.0;
    (normalized.round().min(255.0).max(0.0)) as u8
}

fn apply_white_balance_correction(r: u16, g: u16, b: u16, wb_r: u16, wb_g: u16, wb_b: u16) -> (u8, u8, u8) {
    // Calculate the minimum white balance value to use as baseline
    let min_wb = wb_r.min(wb_g).min(wb_b) as f32;

    // Calculate normalization factors - how much each channel needs to be scaled
    // to balance against the weakest channel
    let r_factor = min_wb / wb_r as f32;
    let g_factor = min_wb / wb_g as f32;
    let b_factor = min_wb / wb_b as f32;

    // Apply the correction factors to the measured values
    let r_corrected = (r as f32 * r_factor).round() as u16;
    let g_corrected = (g as f32 * g_factor).round() as u16;
    let b_corrected = (b as f32 * b_factor).round() as u16;

    // Find the maximum corrected value to determine if scaling is needed
    let max_corrected = r_corrected.max(g_corrected).max(b_corrected);

    // Only scale to 0-255 range if any value exceeds 255
    let (r_final, g_final, b_final) = if max_corrected > 255 {
        let scale_factor = 255.0 / max_corrected as f32;
        let r_scaled = ((r_corrected as f32 * scale_factor).round().min(255.0).max(0.0)) as u8;
        let g_scaled = ((g_corrected as f32 * scale_factor).round().min(255.0).max(0.0)) as u8;
        let b_scaled = ((b_corrected as f32 * scale_factor).round().min(255.0).max(0.0)) as u8;
        (r_scaled, g_scaled, b_scaled)
    } else {
        // Keep original corrected values if all are within 0-255 range
        (r_corrected as u8, g_corrected as u8, b_corrected as u8)
    };

    log::info!("WB correction: R({}->{}) G({}->{}) B({}->{}), factors: R={:.3} G={:.3} B={:.3}, max={}, scaled={}",
               r, r_final, g, g_final, b, b_final, r_factor, g_factor, b_factor, max_corrected, max_corrected > 255);

    (r_final, g_final, b_final)
}

fn apply_brightness_correction(
    r: u16, g: u16, b: u16,
    wb_r: u16, wb_g: u16, wb_b: u16,
    clear: u16, wb_clear: u16
) -> (u8, u8, u8) {
    // First apply white balance correction
    let (r_wb, g_wb, b_wb) = apply_white_balance_correction(r, g, b, wb_r, wb_g, wb_b);

    // Calculate transmission ratio from clear channel
    let transmission_ratio = if wb_clear > 0 {
        (clear as f32 / wb_clear as f32).min(1.0).max(0.01) // Clamp between 1% and 100%
    } else {
        1.0
    };

    log::info!("Transmission ratio: {:.3} (clear: {}, wb_clear: {})",
               transmission_ratio, clear, wb_clear);

    // If transmission is very low, the plastic is very dark/thick
    // We need to boost the color intensity to compensate
    let brightness_boost = if transmission_ratio < 0.1 {
        // For very dark plastic (< 10% transmission), apply strong boost
        1.0 / transmission_ratio.max(0.05) // Cap boost at 20x
    } else if transmission_ratio < 0.5 {
        // For medium darkness (10-50% transmission), apply moderate boost
        1.0 / transmission_ratio.powf(0.7) // Less aggressive boost
    } else {
        // For light plastic (> 50% transmission), minimal or no boost
        1.0 / transmission_ratio.powf(0.3)
    };

    // Apply brightness correction while preserving color ratios
    let r_corrected = (r_wb as f32 * brightness_boost).round().min(255.0).max(0.0) as u8;
    let g_corrected = (g_wb as f32 * brightness_boost).round().min(255.0).max(0.0) as u8;
    let b_corrected = (b_wb as f32 * brightness_boost).round().min(255.0).max(0.0) as u8;

    log::info!("Brightness correction: boost={:.2}, RGB({},{},{}) -> ({},{},{})",
               brightness_boost, r_wb, g_wb, b_wb, r_corrected, g_corrected, b_corrected);

    (r_corrected, g_corrected, b_corrected)
}

fn apply_adaptive_brightness_correction(
    r: u16, g: u16, b: u16, clear: u16,
    wb_r: u16, wb_g: u16, wb_b: u16, wb_clear: u16
) -> (u8, u8, u8) {
    // Apply white balance correction only - no aggressive brightness correction
    let (r_wb, g_wb, b_wb) = apply_white_balance_correction(r, g, b, wb_r, wb_g, wb_b);

    log::info!("Raw RGB: ({},{},{}), WB corrected: ({},{},{}), Clear: {}, WB Clear est: {}",
               r, g, b, r_wb, g_wb, b_wb, clear, wb_clear);

    // Check if the values are already reasonable after white balance
    let max_val = r_wb.max(g_wb).max(b_wb);

    // Only apply very minimal correction if all values are extremely low
    if max_val < 10 {
        // Apply a very conservative boost only for extremely dark readings
        let gentle_boost = 2.0; // Maximum 2x boost

        let r_final = (r_wb as f32 * gentle_boost).round().min(255.0).max(0.0) as u8;
        let g_final = (g_wb as f32 * gentle_boost).round().min(255.0).max(0.0) as u8;
        let b_final = (b_wb as f32 * gentle_boost).round().min(255.0).max(0.0) as u8;

        log::info!("Applied minimal boost for very dark reading: {:.2} -> RGB({},{},{})",
                   gentle_boost, r_final, g_final, b_final);

        return (r_final, g_final, b_final);
    }

    // For all other cases, just use the white balance corrected values
    log::info!("Using white balance corrected values directly: RGB({},{},{})",
               r_wb, g_wb, b_wb);

    (r_wb, g_wb, b_wb)
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
) -> Option<String> {


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

    // Get median values for accurate measurement
    let final_median_lux = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.median().unwrap_or(current_reading)
    };

    let (r_median, g_median, b_median) = {
        let buffers = rgb_buffers.lock().unwrap();
        (
            buffers.0.median().unwrap_or(rgb_white_balance.0),
            buffers.1.median().unwrap_or(rgb_white_balance.1),
            buffers.2.median().unwrap_or(rgb_white_balance.2),
        )
    };

    // Keep TD calculation exactly as is
    let td_value = (final_median_lux / baseline_reading) * 100.0;
    let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;

    // Read clear channel for brightness correction
    let clear_median = {
        let mut locked_rgb = veml_rgb.lock().unwrap();
        locked_rgb.read_clear().unwrap_or(rgb_white_balance.0)
    };

    // Use a more accurate clear reference estimation based on calibration
    let wb_clear_estimate = (rgb_white_balance.0 + rgb_white_balance.1 + rgb_white_balance.2) as f32 * 1.2;

    log::debug!("RGB medians: R={}, G={}, B={}, Clear={}, WB: ({},{},{}), WB Clear est={:.0}",
               r_median, g_median, b_median, clear_median,
               rgb_white_balance.0, rgb_white_balance.1, rgb_white_balance.2, wb_clear_estimate);

    // Apply the improved color correction
    let (r_norm, g_norm, b_norm) = apply_advanced_color_correction(
        r_median, g_median, b_median, clear_median,
        rgb_white_balance.0, rgb_white_balance.1, rgb_white_balance.2, wb_clear_estimate as u16
    );

    // Create hex color string
    let hex_color = format!("#{:02X}{:02X}{:02X}", r_norm, g_norm, b_norm);

    let ws_message = format!("{:.2},{}", adjusted_td_value, hex_color);

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

    log::info!("Reading: {:.2}, RGB: {} (medians from {} lux, {} RGB samples), Raw RGB: ({},{},{}), Final RGB: ({},{},{}) - Baseline: {:.2}, Lux: {}, Clear: {}",
               adjusted_td_value, hex_color, lux_len, rgb_len,
               r_median, g_median, b_median,
               r_norm, g_norm, b_norm,
               saved_algorithm.b, final_median_lux, clear_median);

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

    // For averaged data, we'll take more intensive sampling
    let sample_count = 100;
    let mut clear_readings = Vec::with_capacity(sample_count);

    // Take many rapid samples to fill the median buffer
    for _ in 0..sample_count {
        let clr = match locked_veml.read_lux() {
            Ok(d) => d,
            Err(e) => {
                log::error!("Failed to read sensor: {:?}", e);
                continue;
            }
        };

        {
            let mut buffer = lux_buffer.lock().unwrap();
            buffer.push(clr as f32);
        }

        // Read RGB values
        let mut locked_rgb = veml_rgb.lock().unwrap();
        if let (Ok(r), Ok(g), Ok(b), Ok(clear)) = (
            locked_rgb.read_red(),
            locked_rgb.read_green(),
            locked_rgb.read_blue(),
            locked_rgb.read_clear()
        ) {
            let mut buffers = rgb_buffers.lock().unwrap();
            buffers.0.push(r);
            buffers.1.push(g);
            buffers.2.push(b);
            clear_readings.push(clear);
        }

        embassy_time::Timer::after_millis(12).await;
    }

    {
        let mut led = led_light.lock().unwrap();
        if let Err(e) = led.set_duty(25) {
            log::error!("Failed to adjust LED duty: {:?}", e);
        }
    }

    // Get median values from the filled buffers
    let median = {
        let buffer = lux_buffer.lock().unwrap();
        buffer.median().unwrap_or(current_reading)
    };

    let (r_median, g_median, b_median) = {
        let buffers = rgb_buffers.lock().unwrap();
        (
            buffers.0.median().unwrap_or(rgb_white_balance.0),
            buffers.1.median().unwrap_or(rgb_white_balance.1),
            buffers.2.median().unwrap_or(rgb_white_balance.2),
        )
    };

    // Calculate clear median
    let clear_median = if !clear_readings.is_empty() {
        clear_readings.sort();
        clear_readings[clear_readings.len() / 2]
    } else {
        rgb_white_balance.0 // Fallback
    };

    // Keep TD calculation exactly as is
    let td_value = (median / baseline_reading) * 10.0;
    let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;

    // Use a more accurate clear reference estimation based on calibration
    let wb_clear_estimate = (rgb_white_balance.0 + rgb_white_balance.1 + rgb_white_balance.2) as f32 * 1.2;

    // Apply the improved color correction
    let (r_norm, g_norm, b_norm) = apply_advanced_color_correction(
        r_median, g_median, b_median, clear_median,
        rgb_white_balance.0, rgb_white_balance.1, rgb_white_balance.2, wb_clear_estimate as u16
    );

    let hex_color = format!("#{:02X}{:02X}{:02X}", r_norm, g_norm, b_norm);

    let ws_message = format!("{:.2},{}", adjusted_td_value, hex_color);

    let (lux_len, rgb_len) = {
        let lux_buf = lux_buffer.lock().unwrap();
        let rgb_buf = rgb_buffers.lock().unwrap();
        (lux_buf.len(), rgb_buf.0.len())
    };

    log::info!("Reading: {:.2}, RGB: {} (medians from {} lux, {} RGB samples), Raw RGB: ({},{},{}), Final RGB: ({},{},{})",
              td_value, hex_color, lux_len, rgb_len, r_median, g_median, b_median, r_norm, g_norm, b_norm);

    Some(ws_message)
}

