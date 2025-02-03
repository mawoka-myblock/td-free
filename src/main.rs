#![feature(iter_intersperse)]

use core::fmt::{Debug, Display};
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use core::time::Duration;

use std::sync::{Arc, Mutex};

use edge_http::io::server::Connection;
use edge_http::io::server::{Handler, Server, DEFAULT_BUF_SIZE};
use edge_http::io::Error as EdgeError;
use edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
use edge_http::{Method as EdgeMethod, DEFAULT_MAX_HEADERS_COUNT};
use edge_nal::TcpBind;
use edge_ws::{FrameHeader, FrameType};

use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;

use embedded_io_async::{Read, Write};

use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvsPartition, NvsDefault};
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

use veml7700::Veml7700;
use wifi::WifiEnum;

// use wifi::get;
// use esp_idf_svc::http::server::ws::EspHttpWsConnection;// mod dns;
mod helpers;
// mod led;
mod routes;
mod wifi;

static INDEX_HTML: &str = include_str!("index.html");

static BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
static RUSTC_VERSION: &str = env!("VERGEN_RUSTC_SEMVER");
static GIT_COMMIT_HASH: &str = env!("VERGEN_GIT_SHA");
static GIT_DESCRIBE: &str = env!("VERGEN_GIT_DESCRIBE");
static GIT_COMMIT_TIMESTAMP: &str = env!("VERGEN_GIT_COMMIT_TIMESTAMP");
static GIT_COMMIT_AUTHOR_NAME: &str = env!("VERGEN_GIT_COMMIT_AUTHOR_NAME");

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

    let config = I2cConfig::new()
        .baudrate(20.kHz().into())
        .timeout(Duration::from_millis(100).into());
    let i2c = I2cDriver::new(peripherals.i2c0, i2c_sda, i2c_scl, &config).unwrap();

    let sysloop = EspSystemEventLoop::take().unwrap();

    let nvs = EspDefaultNvsPartition::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let driver = EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs.clone())).unwrap();
    let mut wifi = AsyncWifi::wrap(driver, sysloop, timer_service).unwrap();

    let veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>> = Arc::new(Mutex::new(Veml7700::new(i2c)));
    led_light.lock().unwrap().set_duty_cycle_fully_on().unwrap();
    FreeRtos.delay_ms(500);
    match veml.lock().unwrap().enable() {
        Ok(()) => (),
        Err(_) => log::error!("VEML failed"),
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
    let arced_nvs = Arc::new(nvs.clone());
    // let cloned_nvs_for_algo = Arc::new(nvs.clone());

    // A little trick so as not to allocate the server on-stack, even temporarily
    // Safe to do as `Server` is just a bunch of `MaybeUninit`s
    let mut server = unsafe { Box::new_uninit().assume_init() };

    // Mount the eventfd VFS subsystem or else `edge-nal-std` won't work
    // Keep the handle alive so that the eventfd FS doesn't get unmounted
    let _eventfd = esp_idf_svc::io::vfs::MountedEventfs::mount(3);

    log::info!("Server created");

    match block_on(run(
        &mut server,
        veml,
        dark_baseline_reading,
        baseline_reading,
        wifi_status,
        led_light,
        arced_nvs,
    )) {
        Ok(_) => (),
        Err(e) => log::error!("block_on: {:?}", e),
    };
    Ok(())
}

pub async fn run<'a>(
    server: &mut WsServer,
    veml: Arc<Mutex<Veml7700<I2cDriver<'a>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
) -> Result<(), anyhow::Error> {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80));

    log::info!("Running HTTP server on {addr}");

    let acceptor = edge_nal_std::Stack::new().bind(addr).await?;

    let handler = WsHandler {
        veml,
        dark_baseline_reading,
        baseline_reading,
        wifi_status,
        led_light,
        nvs,
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
        } else if headers.path == "/" || headers.path == "" {
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
            WsHandler::algorithm_route(&self, headers.path, conn).await?;
        } else if headers.path.starts_with("/wifi") {
            WsHandler::wifi_route(&self, headers.path, conn).await?;
        } else if headers.path.starts_with("/ws") {
            match WsHandler::ws_handler(&self, conn).await {
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
