use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use embedded_hal::{delay::DelayNs as _, pwm::SetDutyCycle};
use esp_idf_svc::{
    hal::{delay::FreeRtos, i2c::I2cDriver, ledc::LedcDriver, reset},
    http::server::{ws::EspHttpWsConnection, EspHttpConnection, Request},
    io::Write,
    nvs::{EspNvsPartition, NvsDefault},
    ws::FrameType,
};
use log::error;
use url::Url;
use veml7700::Veml7700;
use ws2812_esp32_rmt_driver::LedPixelEsp32Rmt;

use crate::{
    helpers,
    led::set_led,
    serve_algo_setup_page, serve_wifi_setup_page,
    wifi::{self, WifiEnum},
};

pub fn wifi_route(
    req: Request<&mut EspHttpConnection<'_>>,
    cloned_nvs: Arc<EspNvsPartition<NvsDefault>>,
) -> Result<(), anyhow::Error> {
    let url = Url::parse(&format!("http://google.com{}", req.uri())).unwrap();
    let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let ssid = url_params.get("ssid");
    let password = url_params.get("password");
    if ssid.is_none() && password.is_none() {
        let saved_ssid =
            wifi::get_wifi_ssid(cloned_nvs.clone().as_ref().clone()).unwrap_or_default();
        req.into_ok_response()?
            .write_all(serve_wifi_setup_page(&saved_ssid, "").as_ref())
            .map(|_| ())?;
        return Ok(());
    }
    if ssid.is_none() {
        req.into_ok_response()?
            .write_all(serve_wifi_setup_page("", "SSID is not set").as_ref())
            .map(|_| ())?;
        return Ok(());
    }
    if password.is_none() {
        req.into_ok_response()?
            .write_all(serve_wifi_setup_page(ssid.unwrap(), "Password is not set").as_ref())
            .map(|_| ())?;
        return Ok(());
    }
    match wifi::save_wifi_creds(
        ssid.unwrap(),
        password.unwrap(),
        cloned_nvs.clone().as_ref().clone(),
    ) {
        Ok(_) => {
            req.into_ok_response()?
                .write_all(
                    serve_wifi_setup_page(
                        ssid.unwrap_or(&String::new()),
                        "Saved successfully, resetting now",
                    )
                    .as_ref(),
                )
                .map(|_| ())?;
            FreeRtos.delay_ms(50);
            reset::restart();
        }
        Err(e) => {
            req.into_ok_response()?
                .write_all(
                    serve_wifi_setup_page(
                        ssid.unwrap_or(&String::new()),
                        "COULD NOT SAVE WIFI CREDENTIALS, resetting now",
                    )
                    .as_ref(),
                )
                .map(|_| ())?;
            error!("{:?}", e);
            FreeRtos.delay_ms(50);
            reset::restart();
        }
    };
}

pub fn algorithm_route(
    req: Request<&mut EspHttpConnection<'_>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
) -> Result<(), anyhow::Error> {
    let url = Url::parse(&format!("http://google.com{}", req.uri())).unwrap();
    let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    let m_value = url_params.get("m");
    let b_value = url_params.get("b");
    if m_value.is_none() && b_value.is_none() {
        let saved_algorithm = helpers::get_saved_algorithm_variables(nvs.as_ref().clone());
        req.into_ok_response()?
            .write_all(serve_algo_setup_page(saved_algorithm.0, saved_algorithm.1).as_ref())
            .map(|_| ())?;
        return Ok(());
    }
    let mod_b_value = b_value
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("0.0".to_string()));
    let mod_m_value = m_value
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("1.0".to_string()));
    match helpers::save_algorithm_variables(&mod_b_value, &mod_m_value, nvs.as_ref().clone()) {
        Ok(_) => {
            req.into_ok_response()?
                .write_all(
                    serve_algo_setup_page(
                        mod_b_value.parse::<f32>().unwrap_or(0.0),
                        mod_m_value.parse::<f32>().unwrap_or(1.0),
                    )
                    .as_ref(),
                )
                .map(|_| ())?;
            return Ok(());
        }
        Err(e) => {
            error!("{:?}", e);
            FreeRtos.delay_ms(50);
            reset::restart();
        }
    };
}

pub fn ws_route(
    ws: &mut EspHttpWsConnection,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>>,
    ws2812: Arc<
        Mutex<
            LedPixelEsp32Rmt<
                'static,
                smart_leds::RGB<u8>,
                ws2812_esp32_rmt_driver::driver::color::LedPixelColorImpl<3, 1, 0, 2, 255>,
            >,
        >,
    >,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'_>>> 
) -> Result<(), anyhow::Error> {
    let mut last_sent = Instant::now();
    let saved_algorithm = helpers::get_saved_algorithm_variables(nvs.as_ref().clone());

    loop {
        if ws.is_closed() {
            break;
        }

        if last_sent.elapsed() >= Duration::from_millis(500) {
            last_sent = Instant::now();
            let reading = veml.lock().unwrap().read_lux().unwrap();

            let ws_message: String;
            if 0.8 < reading / dark_baseline_reading {
                let wifi_stat = wifi_status.lock().unwrap();
                match *wifi_stat {
                    WifiEnum::Connected => set_led(ws2812.clone(), 0, 255, 0),
                    WifiEnum::HotSpot => set_led(ws2812.clone(), 255, 0, 255),
                    WifiEnum::Working => set_led(ws2812.clone(), 255, 255, 0),
                }
                log::info!("No filament!");
                ws_message = "no_filament".to_string()
            } else {
                // set_led(ws2812.clone(), 0, 125, 125);
                log::info!("Filament detected!");
                led_light.lock().unwrap().set_duty_cycle_fully_on().unwrap();
                FreeRtos.delay_ms(2);
                let reading = veml.lock().unwrap().read_lux().unwrap();
                let td_value = (reading / baseline_reading) * 100.0;
                let adjusted_td_value = saved_algorithm.1 * td_value + saved_algorithm.0;
                ws_message = adjusted_td_value.to_string();
                led_light.lock().unwrap().set_duty(25).unwrap();
                log::info!("Reading: {}", td_value);
            }
            if let Err(e) = ws.send(FrameType::Text(false), ws_message.as_ref()) {
                log::error!("Error sending WebSocket message: {:?}", e);
                break;
            }
        }

        FreeRtos.delay_ms(2);
    }

    Ok(())
}
