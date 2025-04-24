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
    hal::{i2c::I2cDriver, ledc::LedcDriver, reset},
    http::{client::EspHttpConnection, Method},
    io::Write as _,
};
use log::{error, info};
use serde::Deserialize;
use serde_json::Value;
use url::Url;
use veml7700::Veml7700;

use crate::{
    helpers::{self, read_spoolman_url, NvsData},
    led::set_led,
    serve_algo_setup_page, serve_wifi_setup_page,
    wifi::{self, WifiEnum},
    EdgeError, LedType, WsHandler, WsHandlerError,
};

static INDEX_HTML: &str = include_str!("index.html");


#[derive(Deserialize, Debug)]
struct SpoolmanFilamentResponse {
    id: u32,
    name: String,
}
impl WsHandler<'_> {
    pub async fn server_index_page<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
    where
        T: Read + Write,
    {
        let spoolman_url = helpers::read_spoolman_url(self.nvs.as_ref().clone());
        let spoolman_available = match spoolman_url.is_some() && !spoolman_url.unwrap().is_empty() {
            true => "true",
            false => "false"
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
        let spoolman_url = read_spoolman_url(self.nvs.as_ref().clone());
        if spoolman_url.is_none() || spoolman_url.clone().unwrap().is_empty() {
            conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Could not read storage."#.as_ref())
                .await?;
            return Ok(());
        }

        let mut client = Client::wrap(EspHttpConnection::new(&Default::default()).unwrap());
        let url = format!("{}/api/v1/filament/{}", spoolman_url.unwrap(), filament_id);
        let payload = format!(r#"{{"extra": {{"td": "{}"}}}}"#, value);
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
        if m_value.is_none()
            && b_value.is_none()
            && threshold_value.is_none()
            && spoolman_value.is_none()
        {
            let saved_algorithm = helpers::get_saved_algorithm_variables(self.nvs.as_ref().clone());
            let saved_spoolman = match helpers::read_spoolman_url(self.nvs.as_ref().clone()) {
                Some(d) => d,
                None => "".to_string(),
            };
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(
                serve_algo_setup_page(
                    saved_algorithm.b,
                    saved_algorithm.m,
                    saved_algorithm.threshold,
                    &saved_spoolman,
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
        let save_spoolman_res =
            helpers::save_spoolman_url(&mod_spoolman_value, self.nvs.as_ref().clone());
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
        let data = read_data(
            self.veml.clone(),
            self.dark_baseline_reading,
            self.baseline_reading,
            self.wifi_status.clone(),
            self.led_light.clone(),
            self.ws2812b.clone(),
            self.saved_algorithm,
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
        let data = read_averaged_data(
            self.veml.clone(),
            self.dark_baseline_reading,
            self.baseline_reading,
            self.wifi_status.clone(),
            self.led_light.clone(),
            self.ws2812b.clone(),
            self.saved_algorithm,
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
        // headers: &edge_http::RequestHeaders<'_, N>,
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

            let td_value = read_data(
                self.veml.clone(),
                self.dark_baseline_reading,
                self.baseline_reading,
                self.wifi_status.clone(),
                self.led_light.clone(),
                self.ws2812b.clone(),
                self.saved_algorithm,
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
            embassy_time::Timer::after_millis(500).await;
        }
    }
}

async fn read_data(
    veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'_>>>,
    ws2812: Arc<Mutex<LedType<'_>>>,
    saved_algorithm: NvsData,
) -> Option<String> {
    let reading = match veml.lock().unwrap().read_lux() {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to read sensor: {:?}", e);
            return None;
        }
    };

    let ws_message: String;
    if reading / dark_baseline_reading > saved_algorithm.threshold {
        let wifi_stat = wifi_status.lock().unwrap();
        match *wifi_stat {
            WifiEnum::Connected => set_led(ws2812.clone(), 0, 255, 0),
            WifiEnum::HotSpot => set_led(ws2812.clone(), 255, 0, 255),
            WifiEnum::Working => set_led(ws2812.clone(), 255, 255, 0),
        }
        log::info!("No filament detected!");
        ws_message = "no_filament".to_string();
    } else {
        log::info!("Filament detected!");
        set_led(ws2812.clone(), 0, 125, 125);
        {
            let mut led = led_light.lock().unwrap();
            if let Err(e) = led.set_duty_cycle_fully_on() {
                log::error!("Failed to set LED duty cycle: {:?}", e);
                return None;
            }
        }
        embassy_time::Timer::after_millis(5).await; // Short delay before measuring again
        let reading = match veml.lock().unwrap().read_lux() {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to read sensor after LED activation: {:?}", e);
                return None;
            }
        };

        let td_value = (reading / baseline_reading) * 100.0;
        let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;
        ws_message = adjusted_td_value.to_string();
        {
            let mut led = led_light.lock().unwrap();
            if let Err(e) = led.set_duty(25) {
                log::error!("Failed to adjust LED duty: {:?}", e);
            }
        }

        log::info!("Reading: {}", td_value);
    }
    Some(ws_message)
}

pub async fn is_filament_inserted_dark(
    veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>>,
    dark_baseline_reading: f32,
    saved_algorithm: NvsData,
) -> Result<bool, ()> {
    let reading = match veml.lock().unwrap().read_lux() {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to read sensor: {:?}", e);
            return Err(());
        }
    };
    Ok(!(reading / dark_baseline_reading > saved_algorithm.threshold))
}

const AVERAGE_SAMPLE_RATE: i32 = 10;
const AVERAGE_SAMPLE_DELAY: u64 = 100;
pub async fn read_averaged_data(
    veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'_>>>,
    ws2812: Arc<Mutex<LedType<'_>>>,
    saved_algorithm: NvsData,
) -> Option<String> {
    let reading = match veml.lock().unwrap().read_lux() {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to read sensor: {:?}", e);
            return None;
        }
    };

    let ws_message: String;
    if reading / dark_baseline_reading > saved_algorithm.threshold {
        let wifi_stat = wifi_status.lock().unwrap();
        match *wifi_stat {
            WifiEnum::Connected => set_led(ws2812.clone(), 0, 255, 0),
            WifiEnum::HotSpot => set_led(ws2812.clone(), 255, 0, 255),
            WifiEnum::Working => set_led(ws2812.clone(), 255, 255, 0),
        }
        log::info!("No filament detected!");
        ws_message = "no_filament".to_string();
    } else {
        log::info!("Filament detected!");
        set_led(ws2812.clone(), 0, 125, 125);
        {
            let mut led = led_light.lock().unwrap();
            if let Err(e) = led.set_duty_cycle_fully_on() {
                log::error!("Failed to set LED duty cycle: {:?}", e);
                return None;
            }
        }
        embassy_time::Timer::after_millis(10).await; // Short delay before measuring again
        let mut readings_summed_up: f32 = 0.0;
        let mut unlocked_veml = veml.lock().unwrap();
        for _ in 0..AVERAGE_SAMPLE_RATE {
            readings_summed_up += match unlocked_veml.read_lux() {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Failed to read sensor after LED activation: {:?}", e);
                    return None;
                }
            };
            embassy_time::Timer::after_millis(AVERAGE_SAMPLE_DELAY).await;
        }
        {
            let mut led = led_light.lock().unwrap();
            if let Err(e) = led.set_duty(25) {
                log::error!("Failed to adjust LED duty: {:?}", e);
            }
        }
        let reading = readings_summed_up / AVERAGE_SAMPLE_RATE as f32;
        let td_value = (reading / baseline_reading) * 10.0;
        let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;
        ws_message = adjusted_td_value.to_string();

        log::info!("Reading: {}", td_value);
    }
    Some(ws_message)
}
