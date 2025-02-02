#![feature(iter_intersperse)]

use core::time::Duration;
use edge_http::io::server::{DefaultServer, Handler};
use edge_http::io::Error as EdgeError;
use edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
use edge_http::Method as EdgeMethod;
use edge_nal::TcpBind;
use edge_ws::{FrameHeader, FrameType};
use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;
use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use futures_lite::future::block_on;
use std::net::SocketAddr;

use core::fmt::{Debug, Display};
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

use edge_http::io::server::Connection;

use embedded_io_async::{Read, Write};
// use wifi::get;
// use esp_idf_svc::http::server::ws::EspHttpWsConnection;// mod dns;
mod helpers;
// mod led;
// mod routes;
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
    // let server_configuration = esp_idf_svc::http::server::Configuration {
    //     stack_size: 10240,

    //     ..Default::default()
    // };

    // let cloned_nvs = Arc::new(nvs.clone());
    // let cloned_nvs_for_algo = Arc::new(nvs.clone());
    let mut server = DefaultServer::new();
    log::info!("Server created");

    match block_on(run(&mut server)) {
        Ok(_) => (),
        Err(e) => log::error!("block_on: {:?}", e),
    };
    Ok(())
}

pub async fn run(server: &mut DefaultServer) -> Result<(), anyhow::Error> {
    let addr: SocketAddr = match "0.0.0.0:8881".parse() {
        Ok(d) => d,
        Err(e) => {
            log::error!("socket_addr: {:?}", e);
            return Ok(());
        }
    };

    log::info!("Running HTTP server on {addr}");

    let acceptor = edge_nal_std::Stack::new()
        .bind(addr)
        .await?;

    match server.run(None, acceptor, WsHandler).await {
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

struct WsHandler;

impl Handler for WsHandler {
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
        let headers = conn.headers()?;

        if headers.method != EdgeMethod::Get {
            conn.initiate_response(405, Some("Method Not Allowed"), &[])
                .await?;
        } else if headers.path != "/" {
            conn.initiate_response(404, Some("Not Found"), &[]).await?;
        } else if !conn.is_ws_upgrade_request()? {
            conn.initiate_response(200, Some("OK"), &[("Content-Type", "text/plain")])
                .await?;

            conn.write_all(b"Initiate WS Upgrade request to switch this connection to WS")
                .await?;
        } else {
            let mut buf = [0_u8; MAX_BASE64_KEY_RESPONSE_LEN];
            conn.initiate_ws_upgrade_response(&mut buf).await?;

            conn.complete().await?;

            log::info!("Connection upgraded to WS, starting a simple WS echo server now");

            // Now we have the TCP socket in a state where it can be operated as a WS connection
            // Run a simple WS echo server here

            let mut socket = conn.unbind()?;

            let mut buf = [0_u8; 8192];

            loop {
                let mut header = FrameHeader::recv(&mut socket)
                    .await
                    .map_err(WsHandlerError::Ws)?;
                let payload = header
                    .recv_payload(&mut socket, &mut buf)
                    .await
                    .map_err(WsHandlerError::Ws)?;

                match header.frame_type {
                    FrameType::Text(_) => {
                        log::info!(
                            "Got {header}, with payload \"{}\"",
                            core::str::from_utf8(payload).unwrap()
                        );
                    }
                    FrameType::Binary(_) => {
                        log::info!("Got {header}, with payload {payload:?}");
                    }
                    FrameType::Close => {
                        log::info!("Got {header}, client closed the connection cleanly");
                        break;
                    }
                    _ => {
                        log::info!("Got {header}");
                    }
                }

                // Echo it back now

                header.mask_key = None; // Servers never mask the payload

                if matches!(header.frame_type, FrameType::Ping) {
                    header.frame_type = FrameType::Pong;
                }

                log::info!("Echoing back as {header}");

                header.send(&mut socket).await.map_err(WsHandlerError::Ws)?;
                header
                    .send_payload(&mut socket, payload)
                    .await
                    .map_err(WsHandlerError::Ws)?;
            }
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
