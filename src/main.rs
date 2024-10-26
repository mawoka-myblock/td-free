#![feature(iter_intersperse)]

use std::borrow::Cow;
use std::collections::HashMap;

use core::time::Duration;
use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;
use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::reset;
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::http::Method;
use esp_idf_svc::io::{vfs, Write};

use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::wifi::{AsyncWifi, EspWifi};
use esp_idf_svc::ws::FrameType;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        delay::FreeRtos,
        i2c::{I2cConfig, I2cDriver},
        peripherals::Peripherals,
        prelude::*,
    },
};
use led::set_led;
use log::error;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use url::Url;
use veml7700::Veml7700;
use wifi::WifiEnum;
// use wifi::get;
use esp_idf_svc::http::server::ws::EspHttpWsConnection;
use esp_idf_svc::sys::EspError;
use ws2812_esp32_rmt_driver::{driver::color::LedPixelColorGrb24, LedPixelEsp32Rmt, RGB8};
// mod dns;
mod helpers;
mod led;
mod wifi;

static INDEX_HTML: &str = include_str!("index.html");
#[cfg(feature = "ota")]
static UPDATE_HTML: &str = include_str!("update_page.html");

static BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
static RUSTC_VERSION: &str = env!("VERGEN_RUSTC_SEMVER");
static GIT_COMMIT_HASH: &str = env!("VERGEN_GIT_SHA");
static GIT_DESCRIBE: &str = env!("VERGEN_GIT_DESCRIBE");
static GIT_COMMIT_TIMESTAMP: &str = env!("VERGEN_GIT_COMMIT_TIMESTAMP");
static GIT_COMMIT_AUTHOR_NAME: &str = env!("VERGEN_GIT_COMMIT_AUTHOR_NAME");

pub type LedType<'a> = LedPixelEsp32Rmt<'static, RGB8, LedPixelColorGrb24>;
fn main() -> Result<(), ()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();
    log::info!(
        "Basic init done. Built on {} with Rustc {} from Commit {}, described as \"{}\" and commited on {} by {}.",
        &BUILD_TIMESTAMP,
        &RUSTC_VERSION,
        &GIT_COMMIT_HASH,
        &GIT_DESCRIBE,
        &GIT_COMMIT_TIMESTAMP,
        &GIT_COMMIT_AUTHOR_NAME
    );

    let peripherals = Peripherals::take().unwrap();

    let i2c_sda = peripherals.pins.gpio8;
    let i2c_scl = peripherals.pins.gpio10;
    let rgb_led_pin = peripherals.pins.gpio21;

    let light_timer_driver = LedcTimerDriver::new(
        peripherals.ledc.timer1,
        &TimerConfig::default().frequency(110.Hz().into()),
    )
    .unwrap();
    let led_light = Arc::new(Mutex::new(
        LedcDriver::new(
            peripherals.ledc.channel1,
            light_timer_driver,
            peripherals.pins.gpio7,
        )
        .unwrap(),
    ));

    let rgb_led_channel = peripherals.rmt.channel0;
    let ws2812 = Arc::new(Mutex::new(
        LedType::new(rgb_led_channel, rgb_led_pin).unwrap(),
    ));
    let pixels = std::iter::repeat(RGB8::new(255, 255, 0)).take(1);
    ws2812.lock().unwrap().write_nocopy(pixels).unwrap();

    let config = I2cConfig::new()
        .baudrate(20.kHz().into())
        .timeout(Duration::from_millis(100).into());
    let i2c = I2cDriver::new(peripherals.i2c0, i2c_sda, i2c_scl, &config).unwrap();

    let sysloop = EspSystemEventLoop::take().unwrap();

    vfs::initialize_eventfd(5).unwrap();

    let nvs = EspDefaultNvsPartition::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let driver = EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs.clone())).unwrap();
    let mut wifi = AsyncWifi::wrap(driver, sysloop, timer_service).unwrap();

    let veml = Arc::new(Mutex::new(Veml7700::new(i2c)));
    led_light.lock().unwrap().set_duty_cycle_fully_on().unwrap();
    FreeRtos.delay_ms(500);
    match veml.lock().unwrap().enable() {
        Ok(()) => (),
        Err(_) => led::show_veml_not_found_error(ws2812.clone()),
    };

    let baseline_reading: f32 = take_baseline_reading(veml.clone());
    led_light.lock().unwrap().set_duty(25).unwrap();
    FreeRtos.delay_ms(200);
    let dark_baseline_reading: f32 = take_baseline_reading(veml.clone());
    log::info!("Baseline readings completed");
    let wifi_status = Arc::new(Mutex::new(WifiEnum::Working));

    block_on(wifi::wifi_setup(
        &mut wifi,
        nvs.clone(),
        ws2812.clone(),
        wifi_status.clone(),
    ))
    .unwrap();
    log::info!("WiFi Started");
    /*
       thread::Builder::new()
           .stack_size(60 * 1024)
           .spawn(|| smol_block_on(start_dns()).unwrap())
           .unwrap();
       log::info!("DNS Started");
    */
    let server_configuration = esp_idf_svc::http::server::Configuration {
        stack_size: 10240,

        ..Default::default()
    };

    let mut server = EspHttpServer::new(&server_configuration).unwrap();
    let cloned_nvs = Arc::new(nvs.clone());
    let cloned_nvs_for_algo = Arc::new(nvs.clone());

    server
        .fn_handler("/", Method::Get, |req| {
            req.into_ok_response()?
                .write_all(INDEX_HTML.as_bytes())
                .map(|_| ())
        })
        .unwrap();

    server
        .fn_handler::<anyhow::Error, _>("/wifi", Method::Get, move |req| {
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
        })
        .unwrap();
    server
        .fn_handler::<anyhow::Error, _>("/algorithm", Method::Get, move |req| {
            let url = Url::parse(&format!("http://google.com{}", req.uri())).unwrap();
            let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
            let m_value = url_params.get("m");
            let b_value = url_params.get("b");
            if m_value.is_none() && b_value.is_none() {
                let saved_algorithm =
                    helpers::get_saved_algorithm_variables(cloned_nvs_for_algo.as_ref().clone());
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
            match helpers::save_algorithm_variables(
                &mod_b_value,
                &mod_m_value,
                cloned_nvs_for_algo.as_ref().clone(),
            ) {
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
        })
        .unwrap();

    #[cfg(feature = "ota")]
    server
        .fn_handler::<anyhow::Error, _>("/update", Method::Post, move |mut req| {
            let mut ota = esp_ota::OtaUpdate::begin()?;
            let mut update_buffer: Vec<u8> = vec![];
            req.read(&mut update_buffer)?;
            ota.write(&mut update_buffer)?;
            let mut completed_update = ota.finalize()?;
            completed_update.set_as_boot_partition()?;
            log::info!("Update successful, restarting!");
            completed_update.restart();
        })
        .unwrap();
    #[cfg(feature = "ota")]
    server
        .fn_handler::<anyhow::Error, _>("/update", Method::Get, move |req| {
            req.into_ok_response()?.write_all(UPDATE_HTML.as_bytes())?;
            Ok(())
        })
        .unwrap();

    let cloned_nvs_for_ws = Arc::new(nvs.clone());
    server
        .ws_handler("/ws", move |ws: &mut EspHttpWsConnection| {
            let mut last_sent = Instant::now();
            let saved_algorithm =
                helpers::get_saved_algorithm_variables(cloned_nvs_for_ws.as_ref().clone());

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
                        set_led(ws2812.clone(), 0, 125, 125);
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

            Ok::<(), EspError>(())
        })
        .unwrap();

    log::info!("Finished setup!");
    loop {
        FreeRtos.delay_ms(500u32);
    }
}

fn serve_wifi_setup_page(current_ssid: &str, error: &str) -> String {
    format!(
        include_str!("wifi_setup.html"),
        ssid = current_ssid,
        error = error
    )
}

fn serve_algo_setup_page(b_val: f32, m_val: f32) -> String {
    format!(
        include_str!("algorithm_setup.html"),
        b_val = b_val,
        m_val = m_val
    )
}
/*
async fn start_dns() -> anyhow::Result<()> {
    let stack = edge_nal_std::Stack::new();
    let mut tx = [0; 1500];
    let mut rx = [0; 1500];

    edge_captive::io::run(
        &stack,
        std::net::SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 53)),
        &mut tx,
        &mut rx,
        Ipv4Addr::new(192, 168, 71, 1),
        Duration::from_secs(60),
    )
    .await?;
    Ok(())
}
     */

fn take_baseline_reading(veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>>) -> f32 {
    let mut max_reading: f32 = 0f32;
    let sample_count = 10;
    let sample_delay = 200u32;

    for _ in 0..sample_count {
        let reading = match veml.lock().unwrap().read_lux() {
            Ok(d) => d,
            Err(e) => {
                log::error!("{:?}", e);
                veml.lock().unwrap().disable().unwrap();
                FreeRtos.delay_ms(100);
                veml.lock().unwrap().enable().unwrap();
                FreeRtos.delay_ms(1000);
                continue;
            }
        };
        log::info!("Reading: {}", reading.clone());
        max_reading += reading;
        FreeRtos.delay_ms(sample_delay);
    }
    max_reading / sample_count as f32
}
