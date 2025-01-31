#![feature(iter_intersperse)]

use core::time::Duration;
use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;
use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::http::Method;
use esp_idf_svc::io::{vfs, Write};

use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::wifi::{AsyncWifi, EspWifi};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        delay::FreeRtos,
        i2c::{I2cConfig, I2cDriver},
        peripherals::Peripherals,
        prelude::*,
    },
};
use std::sync::{Arc, Mutex};
use veml7700::Veml7700;
use wifi::WifiEnum;
// use wifi::get;
use esp_idf_svc::http::server::ws::EspHttpWsConnection;
use ws2812_esp32_rmt_driver::{driver::color::LedPixelColorGrb24, LedPixelEsp32Rmt, RGB8};
// mod dns;
mod helpers;
mod led;
mod routes;
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
    let led_light: Arc<Mutex<LedcDriver<'_>>> = Arc::new(Mutex::new(
        LedcDriver::new(
            peripherals.ledc.channel1,
            light_timer_driver,
            peripherals.pins.gpio7,
        )
        .unwrap(),
    ));

    let rgb_led_channel = peripherals.rmt.channel0;
    let ws2812: Arc<
        Mutex<
            LedPixelEsp32Rmt<
                '_,
                smart_leds::RGB<u8>,
                ws2812_esp32_rmt_driver::driver::color::LedPixelColorImpl<3, 1, 0, 2, 255>,
            >,
        >,
    > = Arc::new(Mutex::new(
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

    let veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>> = Arc::new(Mutex::new(Veml7700::new(i2c)));
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
    let wifi_status: Arc<Mutex<WifiEnum>> = Arc::new(Mutex::new(WifiEnum::Working));

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
                .write_all(
                    INDEX_HTML
                        .replace(
                            "{{VERSION}}",
                            option_env!("TD_FREE_VERSION").unwrap_or("UNKNOWN"),
                        )
                        .as_bytes(),
                )
                .map(|_| ())
        })
        .unwrap();

    server
        .fn_handler::<anyhow::Error, _>("/wifi", Method::Get, move |req| {
            routes::wifi_route(req, cloned_nvs.clone())
        })
        .unwrap();
    server
        .fn_handler::<anyhow::Error, _>("/algorithm", Method::Get, move |req| {
            routes::algorithm_route(req, cloned_nvs_for_algo.clone())
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
            routes::ws_route(
                ws,
                cloned_nvs_for_ws.clone(),
                dark_baseline_reading,
                baseline_reading,
                veml.clone(),
                ws2812.clone(),
                wifi_status.clone(),
                led_light.clone(),
            )
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
