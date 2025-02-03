use std::{
    borrow::Cow,
    collections::HashMap,
    str,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use edge_http::io::{server::Connection, Error};
use edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
use edge_ws::{FrameHeader, FrameType};
use embedded_hal::{delay::DelayNs as _, pwm::SetDutyCycle};
use embedded_io_async::{Read, Write};
use esp_idf_svc::{
    hal::{delay::FreeRtos, i2c::I2cDriver, ledc::LedcDriver, reset},
    nvs::{EspNvsPartition, NvsDefault},
};
use log::error;
use url::Url;
use veml7700::Veml7700;

use crate::{
    helpers, serve_algo_setup_page, serve_wifi_setup_page,
    wifi::{self, WifiEnum},
    EdgeError, WsHandler, WsHandlerError,
};

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
                FreeRtos.delay_ms(50);
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
                FreeRtos.delay_ms(50);
                reset::restart();
            }
        };
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
        if m_value.is_none() && b_value.is_none() {
            let saved_algorithm = helpers::get_saved_algorithm_variables(self.nvs.as_ref().clone());
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(serve_algo_setup_page(saved_algorithm.0, saved_algorithm.1).as_ref())
                .await?;
            return Ok(());
        }
        let mod_b_value = b_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("0.0".to_string()));
        let mod_m_value = m_value
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned("1.0".to_string()));
        match helpers::save_algorithm_variables(
            &mod_b_value,
            &mod_m_value,
            self.nvs.as_ref().clone(),
        ) {
            Ok(_) => {
                conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                    .await?;
                conn.write_all(
                    serve_algo_setup_page(
                        mod_b_value.parse::<f32>().unwrap_or(0.0),
                        mod_m_value.parse::<f32>().unwrap_or(1.0),
                    )
                    .as_ref(),
                )
                .await?;
                return Ok(());
            }
            Err(e) => {
                error!("{:?}", e);
                FreeRtos.delay_ms(50);
                reset::restart();
            }
        };
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
        let saved_algorithm = helpers::get_saved_algorithm_variables(self.nvs.as_ref().clone());
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
                saved_algorithm,
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

        Ok(())
    }
}

async fn read_data(
    veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'_>>>,
    saved_algorithm: (f32, f32),
) -> Option<String> {
    let reading = match veml.lock().unwrap().read_lux() {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to read sensor: {:?}", e);
            return None;
        }
    };

    let ws_message: String;
    if reading / dark_baseline_reading > 0.8 {
        let wifi_stat = wifi_status.lock().unwrap();
        // match *wifi_stat {
        //     WifiEnum::Connected => set_led(ws2812.clone(), 0, 255, 0),
        //     WifiEnum::HotSpot => set_led(ws2812.clone(), 255, 0, 255),
        //     WifiEnum::Working => set_led(ws2812.clone(), 255, 255, 0),
        // }
        log::info!("No filament detected!");
        ws_message = "no_filament".to_string();
    } else {
        log::info!("Filament detected!");
        let mut led = led_light.lock().unwrap();
        if let Err(e) = led.set_duty_cycle_fully_on() {
            log::error!("Failed to set LED duty cycle: {:?}", e);
            return None;
        }

        FreeRtos::delay_ms(2); // Short delay before measuring again
        let reading = match veml.lock().unwrap().read_lux() {
            Ok(r) => r,
            Err(e) => {
                log::error!("Failed to read sensor after LED activation: {:?}", e);
                return None;
            }
        };

        let td_value = (reading / baseline_reading) * 100.0;
        let adjusted_td_value = saved_algorithm.1 * td_value + saved_algorithm.0;
        ws_message = adjusted_td_value.to_string();

        if let Err(e) = led.set_duty(25) {
            log::error!("Failed to adjust LED duty: {:?}", e);
        }

        log::info!("Reading: {}", td_value);
    }
    return Some(ws_message);
}
