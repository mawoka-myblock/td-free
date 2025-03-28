#![feature(iter_intersperse)]

use core::fmt::{Debug, Display};
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use core::time::Duration;

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::{future, str};

use edge_http::io::server::Connection;
use edge_http::io::server::{Handler, Server, DEFAULT_BUF_SIZE};
use edge_http::io::Error as EdgeError;
use edge_http::Method as EdgeMethod;
use edge_nal::TcpBind;

use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;

use embedded_io_async::{Read, Write};

use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::usb_serial::{UsbSerialConfig, UsbSerialDriver};
use esp_idf_svc::io::Write as ioWrite;
use esp_idf_svc::ipv4::{
    self, ClientConfiguration as IpClientConfiguration, Configuration as IpConfiguration,
    DHCPClientSettings, Mask, RouterConfiguration, Subnet,
};
use esp_idf_svc::netif::{EspNetif, NetifConfiguration};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvsPartition, NvsDefault};
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::wifi::{AsyncWifi, EspWifi, WifiDriver};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{
        delay::FreeRtos,
        i2c::{I2cConfig, I2cDriver},
        peripherals::Peripherals,
        prelude::*,
    },
};

use helpers::NvsData;
use led::set_led;
use smart_leds::RGB8;
use veml7700::Veml7700;
use wifi::WifiEnum;
use ws2812_esp32_rmt_driver::driver::color::LedPixelColorGrb24;
use ws2812_esp32_rmt_driver::LedPixelEsp32Rmt;

mod helpers;
mod led;
mod routes;
mod wifi;

static INDEX_HTML: &str = include_str!("index.html");

static BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
static RUSTC_VERSION: &str = env!("VERGEN_RUSTC_SEMVER");
static GIT_COMMIT_HASH: &str = env!("VERGEN_GIT_SHA");
static GIT_DESCRIBE: &str = env!("VERGEN_GIT_DESCRIBE");
static GIT_COMMIT_TIMESTAMP: &str = env!("VERGEN_GIT_COMMIT_TIMESTAMP");
static GIT_COMMIT_AUTHOR_NAME: &str = env!("VERGEN_GIT_COMMIT_AUTHOR_NAME");

pub const IP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 71, 1);
pub type LedType<'a> = LedPixelEsp32Rmt<'static, RGB8, LedPixelColorGrb24>;

fn main() -> Result<(), ()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    let logger = esp_idf_svc::log::EspLogger::new();
    logger
        .set_target_level("*", log::LevelFilter::Trace)
        .unwrap();
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
        &TimerConfig::default().frequency(110.Hz()),
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

    let nvs = EspDefaultNvsPartition::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    // let driver = EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs.clone())).unwrap();
    let wifi_raw_driver =
        WifiDriver::new(peripherals.modem, sysloop.clone(), Some(nvs.clone())).unwrap();
    let driver = EspWifi::wrap_all(
        wifi_raw_driver,
        EspNetif::new_with_conf(&NetifConfiguration {
            ip_configuration: Some(IpConfiguration::Client(IpClientConfiguration::DHCP(
                DHCPClientSettings {
                    hostname: Some("tdfree".try_into().unwrap()),
                },
            ))),
            ..NetifConfiguration::wifi_default_client()
        })
        .unwrap(),
        EspNetif::new_with_conf(&NetifConfiguration {
            ip_configuration: Some(ipv4::Configuration::Router(RouterConfiguration {
                subnet: Subnet {
                    gateway: IP_ADDRESS,
                    mask: Mask(24),
                },
                dhcp_enabled: true,
                dns: Some(IP_ADDRESS),
                secondary_dns: Some(IP_ADDRESS),
            })),
            ..NetifConfiguration::wifi_default_router()
        })
        .unwrap(),
    )
    .unwrap();
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

    let hotspot_ip = block_on(wifi::wifi_setup(
        &mut wifi,
        nvs.clone(),
        ws2812.clone(),
        wifi_status.clone(),
    ))
    .unwrap()
    .1;
    log::info!("WiFi Started");
    /*
       thread::Builder::new()
           .stack_size(60 * 1024)
           .spawn(|| smol_block_on(start_dns()).unwrap())
           .unwrap();
       log::info!("DNS Started");
    */
    let arced_nvs = Arc::new(nvs.clone());
    // let cloned_nvs_for_algo = Arc::new(nvs.clone());

    // A little trick so as not to allocate the server on-stack, even temporarily
    // Safe to do as `Server` is just a bunch of `MaybeUninit`s
    let mut server = unsafe { Box::new_uninit().assume_init() };

    // Mount the eventfd VFS subsystem or else `edge-nal-std` won't work
    // Keep the handle alive so that the eventfd FS doesn't get unmounted
    let _eventfd = esp_idf_svc::io::vfs::MountedEventfs::mount(3);
    let saved_algorithm = helpers::get_saved_algorithm_variables(arced_nvs.as_ref().clone());

    log::info!("Server created");
    let stack = edge_nal_std::Stack::new();
    let server_future = run(
        &mut server,
        veml.clone(),
        dark_baseline_reading,
        baseline_reading,
        wifi_status.clone(),
        led_light.clone(),
        arced_nvs.clone(),
        &stack,
        ws2812.clone(),
        saved_algorithm,
    );
    log::warn!(
        "Activating serial connection, hold e to keep viewing logs and disable serial interface!!!"
    );
    let mut serial_driver = UsbSerialDriver::new(
        peripherals.usb_serial,
        peripherals.pins.gpio18,
        peripherals.pins.gpio19,
        &UsbSerialConfig::new(),
    )
    .unwrap();
    let mut exit_buffer = [0u8; 1];
    serial_driver.read(&mut exit_buffer, 500).unwrap();
    let serial_future = async move {
        if exit_buffer.iter().any(|&x| x == b'e') {
            drop(serial_driver);
            log::info!("Logging reactivated!");
            future::pending::<Result<(), anyhow::Error>>().await
        } else {
            log::warn!("Logging deactivated from now on, this is last log message!");
            logger.set_target_level("*", log::LevelFilter::Off).unwrap();
            serial_connection(
                &mut serial_driver,
                veml,
                dark_baseline_reading,
                baseline_reading,
                wifi_status,
                led_light,
                ws2812.clone(),
                saved_algorithm,
            )
            .await
        }
    };
    // let serial_future = serial_connection(&mut serial_driver);

    if hotspot_ip.is_some() {
        log::info!("Running with captive portal");
        let mut tx_buf = [0; 1500];
        let mut rx_buf = [0; 1500];
        let captive_future = edge_captive::io::run(
            &stack,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 53),
            &mut tx_buf,
            &mut rx_buf,
            IP_ADDRESS,
            Duration::from_secs(60),
        );
        let block_on_res = block_on(embassy_futures::join::join3(
            server_future,
            captive_future,
            serial_future,
        ));
        match block_on_res.0 {
            Ok(_) => (),
            Err(e) => log::error!("Server error: {:?}", e),
        };
        match block_on_res.1 {
            Ok(_) => (),
            Err(e) => log::error!("Captive Portal error: {:?}", e),
        };
        match block_on_res.2 {
            Ok(_) => log::info!("Logging reactivated!"),
            Err(e) => log::error!("Serial error: {:?}", e),
        };
    } else {
        log::info!("Running without captive portal");
        let block_on_res = block_on(embassy_futures::join::join(server_future, serial_future));
        match block_on_res.0 {
            Ok(_) => (),
            Err(e) => log::error!("Server error: {:?}", e),
        };
        match block_on_res.1 {
            Ok(_) => log::info!("Logging reactivated!"),
            Err(e) => log::error!("Serial error: {:?}", e),
        };
    }

    Ok(())
}

pub async fn serial_connection<'a>(
    conn: &mut UsbSerialDriver<'static>,
    veml: Arc<Mutex<Veml7700<I2cDriver<'a>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    ws2812: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
) -> Result<(), anyhow::Error> {
    let mut buffer = [0u8; 64]; // Buffer for reading incoming data

    loop {
        // Step 1: Wait for "connect\n" from Python script
        let n = match conn.read(&mut buffer, 500) {
            Ok(n) if n > 0 => n,
            _ => continue, // No data, continue looping
        };

        let received = core::str::from_utf8(&buffer[..n]).unwrap_or("").trim();

        if received == "connect" {
            set_led(ws2812.clone(), 30, 30, 100);
            // Step 2: Respond with "ready"
            conn.write(b"ready\n", 100).unwrap();
            conn.flush().unwrap();
            conn.write(b"ready\n", 100).unwrap();
            conn.flush().unwrap();
        }

        if received == "P" {
            set_led(ws2812.clone(), 100, 30, 255);
            embassy_time::Timer::after_millis(500).await;
            loop {
                let is_filament_inserted = routes::is_filament_inserted_dark(
                    veml.clone(),
                    dark_baseline_reading,
                    saved_algorithm,
                )
                .await
                .unwrap();
                if !is_filament_inserted {
                    embassy_time::Timer::after_millis(100).await;
                    continue;
                }
                set_led(ws2812.clone(), 0, 125, 125);
                embassy_time::Timer::after_millis(300).await;

                let reading = routes::read_averaged_data(
                    veml.clone(),
                    dark_baseline_reading,
                    baseline_reading,
                    wifi_status.clone(),
                    led_light.clone(),
                    ws2812.clone(),
                    saved_algorithm,
                )
                .await;

                let message = format!(
                    "uid,brand,type,name,{},color,owned,uuid\n",
                    reading.unwrap_or("unknown".to_string())
                );

                conn.write(message.as_bytes(), 100).unwrap();
                conn.flush().unwrap();
                embassy_time::Timer::after_millis(300).await;
                loop {
                    embassy_time::Timer::after_millis(150).await;
                    let is_filament_inserted = routes::is_filament_inserted_dark(
                        veml.clone(),
                        dark_baseline_reading,
                        saved_algorithm,
                    )
                    .await
                    .unwrap();
                    if !is_filament_inserted {
                        break;
                    }
                }
                set_led(ws2812.clone(), 100, 30, 255);

                embassy_time::Timer::after_millis(100).await; // Prevent excessive polling
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run<'a>(
    server: &mut WsServer,
    veml: Arc<Mutex<Veml7700<I2cDriver<'a>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    stack: &edge_nal_std::Stack,
    ws2812b: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
) -> Result<(), anyhow::Error> {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80));

    log::info!("Running HTTP server on {addr}");

    let acceptor = stack.bind(addr).await?;

    let handler = WsHandler {
        veml,
        dark_baseline_reading,
        baseline_reading,
        wifi_status,
        led_light,
        nvs,
        ws2812b,
        saved_algorithm,
    };
    match server.run(None, acceptor, handler).await {
        Ok(_) => (),
        Err(e) => log::error!("server.run: {:?}", e),
    };

    Ok(())
}

#[derive(Debug)]
enum WsHandlerError<C, W> {
    Connection(C),
    Ws(W),
}

impl<C, W> From<C> for WsHandlerError<C, W> {
    fn from(e: C) -> Self {
        Self::Connection(e)
    }
}

// Reduce the size of the future by using max 2 handler instead of 4
// and by limiting the number of headers to 32 instead of 64
type WsServer = Server<2, { DEFAULT_BUF_SIZE }, 32>;

struct WsHandler<'a> {
    veml: Arc<Mutex<Veml7700<I2cDriver<'a>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    ws2812b: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
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

        if headers.method != EdgeMethod::Get {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path == "/" || headers.path.is_empty() {
            conn.initiate_response(200, None, &[("Content-Type", "text/html")])
                .await?;
            conn.write_all(
                INDEX_HTML
                    .replace(
                        "{{VERSION}}",
                        option_env!("TD_FREE_VERSION").unwrap_or("UNKNOWN"),
                    )
                    .as_bytes(),
            )
            .await?;
        } else if headers.path.starts_with("/algorithm") {
            WsHandler::algorithm_route(self, headers.path, conn).await?;
        } else if headers.path.starts_with("/wifi") {
            WsHandler::wifi_route(self, headers.path, conn).await?;
        } else if headers.path.starts_with("/fallback") {
            WsHandler::fallback_route(self, conn).await?;
        } else if headers.path.starts_with("/averaged") {
            WsHandler::averaged_reading_route(self, conn).await?;
        } else if headers.path.starts_with("/ws") {
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

fn serve_wifi_setup_page(current_ssid: &str, error: &str) -> String {
    format!(
        include_str!("wifi_setup.html"),
        ssid = current_ssid,
        error = error
    )
}

fn serve_algo_setup_page(b_val: f32, m_val: f32, threshold_val: f32) -> String {
    format!(
        include_str!("algorithm_setup.html"),
        b_val = b_val,
        m_val = m_val,
        threshold_val = threshold_val
    )
}

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
