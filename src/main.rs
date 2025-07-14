#![feature(iter_intersperse)]
#![allow(clippy::await_holding_lock)]

use crate::helpers::baseline_readings::take_baseline_reading;
use crate::helpers::baseline_readings::take_rgb_white_balance_calibration;
use crate::helpers::i2c_init::Pins;
use crate::helpers::i2c_init::initialize_veml;
use crate::helpers::median_buffer;
use crate::helpers::nvs::NvsData;
use crate::helpers::nvs::RGBMultipliers;
use crate::helpers::nvs::clear_rgb_multipliers_nvs;
use crate::helpers::nvs::get_saved_algorithm_variables;
use crate::helpers::nvs::get_saved_rgb_multipliers;
use crate::helpers::serial::serial_connection;
use core::fmt::Debug;
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use std::str;
use std::sync::{Arc, Mutex};

use edge_http::io::Error as EdgeError;
use edge_http::io::server::Server;
use edge_nal::TcpBind;

use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;

use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::usb_serial::{UsbSerialConfig, UsbSerialDriver};
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
    hal::{delay::FreeRtos, peripherals::Peripherals, prelude::*},
};

use helpers::bitbang_i2c::HardwareI2cInstance;
use helpers::bitbang_i2c::SimpleBitBangI2cInstance;
use log::info;
use smart_leds::RGB8;
use veml7700::Veml7700;
use wifi::WifiEnum;
use ws2812_esp32_rmt_driver::LedPixelEsp32Rmt;
use ws2812_esp32_rmt_driver::driver::color::LedPixelColorGrb24;

mod helpers;
mod led;
mod routes;
mod veml3328;
mod wifi;

static BUILD_TIMESTAMP: &str = env!("VERGEN_BUILD_TIMESTAMP");
static RUSTC_VERSION: &str = env!("VERGEN_RUSTC_SEMVER");
static GIT_COMMIT_HASH: &str = env!("VERGEN_GIT_SHA");
static GIT_DESCRIBE: &str = env!("VERGEN_GIT_DESCRIBE");
static GIT_COMMIT_TIMESTAMP: &str = env!("VERGEN_GIT_COMMIT_TIMESTAMP");
static GIT_COMMIT_AUTHOR_NAME: &str = env!("VERGEN_GIT_COMMIT_AUTHOR_NAME");

pub const IP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 71, 1);
pub type LedType<'a> = LedPixelEsp32Rmt<'static, RGB8, LedPixelColorGrb24>;
pub type ArcLed<'a> = Arc<
    Mutex<
        LedPixelEsp32Rmt<
            'a,
            smart_leds::RGB<u8>,
            ws2812_esp32_rmt_driver::driver::color::LedPixelColorImpl<3, 1, 0, 2, 255>,
        >,
    >,
>;

fn main() -> Result<(), ()> {
    // It is necessary to call this function once. Otherwise some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    log::set_max_level(log::LevelFilter::Trace);

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

    let ws2812_old: ArcLed = Arc::new(Mutex::new(
        LedType::new(peripherals.rmt.channel0, peripherals.pins.gpio21).unwrap(),
    ));
    let ws2812_new: ArcLed = Arc::new(Mutex::new(
        LedType::new(peripherals.rmt.channel1, peripherals.pins.gpio4).unwrap(),
    ));
    ws2812_old
        .lock()
        .unwrap()
        .write_nocopy(std::iter::repeat_n(RGB8::new(255, 255, 0), 1))
        .unwrap();
    ws2812_new
        .lock()
        .unwrap()
        .write_nocopy(std::iter::repeat_n(RGB8::new(255, 255, 0), 1))
        .unwrap();
    let veml_res = initialize_veml(
        Pins {
            i2c: peripherals.i2c0,
            sda1: peripherals.pins.gpio6,
            scl1: peripherals.pins.gpio5,
            sda2: peripherals.pins.gpio8,
            scl2: peripherals.pins.gpio10,
        },
        ws2812_old.clone(),
        ws2812_new.clone(),
    );
    info!(
        "Old PCB? {}, Color? {}",
        veml_res.is_old_pcb,
        veml_res.veml3328.is_some()
    );
    let ws2812 = match veml_res.is_old_pcb {
        true => ws2812_old,
        false => ws2812_new,
    };
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
    let wifi = AsyncWifi::wrap(driver, sysloop, timer_service).unwrap();

    let wifi_status: Arc<Mutex<WifiEnum>> = Arc::new(Mutex::new(WifiEnum::Working));
    let wifi = Arc::new(Mutex::new(wifi));

    // Spawn WiFi thread for background management
    let wifi_status_clone = wifi_status.clone();
    let wifi_clone = wifi.clone();
    let ws2812_clone = ws2812.clone();
    let nvs_clone = nvs.clone();

    std::thread::spawn(move || {
        // Run the async wifi thread in a blocking executor
        esp_idf_svc::hal::task::block_on(wifi::wifi_thread(
            wifi_clone,
            nvs_clone,
            ws2812_clone,
            wifi_status_clone,
        ));
    });

    log::info!("WiFi thread started");

    // let veml: Arc<Mutex<VEML3328<I2cDriver<'_>>>> = Arc::new(Mutex::new(VEML3328::new(i2c)));
    led_light.lock().unwrap().set_duty_cycle_fully_on().unwrap();
    FreeRtos.delay_ms(500);
    let baseline_reading: f32 = take_baseline_reading(veml_res.veml7700.clone());

    // White balance calibration at 50% LED brightness
    let rgb_white_balance: Option<(u16, u16, u16)> = match veml_res.veml3328.clone() {
        Some(d) => Some(take_rgb_white_balance_calibration(
            d.clone(),
            led_light.clone(),
        )),
        None => None,
    };

    led_light.lock().unwrap().set_duty(25).unwrap();
    FreeRtos.delay_ms(500);
    let dark_baseline_reading: f32 = take_baseline_reading(veml_res.veml7700.clone());

    // For compatibility, we'll use the white balance as both baseline values
    let dark_rgb_baseline = rgb_white_balance;

    log::info!("Baseline readings completed with white balance calibration");

    let arced_nvs = Arc::new(nvs.clone());

    let mut server = unsafe { Box::new_uninit().assume_init() };

    let _eventfd = esp_idf_svc::io::vfs::MountedEventfs::mount(3);

    // Try to load algorithm variables with error recovery
    let saved_algorithm = match std::panic::catch_unwind(|| {
        get_saved_algorithm_variables(arced_nvs.as_ref().clone())
    }) {
        Ok(algorithm) => algorithm,
        Err(_) => {
            log::error!("Algorithm loading caused panic - using defaults");
            NvsData {
                b: 0.0,
                m: 1.0,
                threshold: 0.5,
            }
        }
    };

    // Try to load RGB multipliers with error recovery and wrap in Arc<Mutex<>>
    let saved_rgb_multipliers = Arc::new(Mutex::new(
        match std::panic::catch_unwind(|| get_saved_rgb_multipliers(arced_nvs.as_ref().clone())) {
            Ok(multipliers) => multipliers,
            Err(_) => {
                log::error!(
                    "RGB multipliers loading caused panic - clearing NVS and using defaults"
                );
                // Clear the corrupted data
                if let Err(e) = clear_rgb_multipliers_nvs(arced_nvs.as_ref().clone()) {
                    log::error!("Failed to clear RGB multipliers NVS: {e:?}");
                }
                RGBMultipliers::default()
            }
        },
    ));

    log::info!("Server created");
    let stack = edge_nal_std::Stack::new();
    let server_data = ServerRunData {
        veml: veml_res.veml7700.clone(),
        veml_rgb: veml_res.veml3328.clone(), // TODO
        dark_baseline_reading,
        baseline_reading,
        rgb_baseline: rgb_white_balance, // Use white balance instead of rgb_baseline TODO
        dark_rgb_baseline,               // TODO
        wifi_status: wifi_status.clone(),
        led_light: led_light.clone(),
        nvs: arced_nvs.clone(),
        ws2812b: ws2812.clone(),
        saved_algorithm,
        saved_rgb_multipliers: *saved_rgb_multipliers.lock().unwrap(),
    };
    let server_future = run(server_data, &stack, &mut server);

    // --- Serial connection setup ---
    let mut serial_driver = UsbSerialDriver::new(
        peripherals.usb_serial,
        peripherals.pins.gpio18,
        peripherals.pins.gpio19,
        &UsbSerialConfig::new(),
    )
    .unwrap();
    let mut exit_buffer = [0u8; 1];
    serial_driver.read(&mut exit_buffer, 500).unwrap();
    let serial_future = {
        async move {
            if exit_buffer.contains(&b'e') {
                drop(serial_driver);
                log::info!("Logging reactivated!");
                std::future::pending::<Result<(), anyhow::Error>>().await
            } else {
                //log::warn!("Logging deactivated from now on, this is last log message!");
                //logger.set_target_level("*", log::LevelFilter::Off).unwrap();
                serial_connection(&mut serial_driver).await
            }
        }
    };

    // --- Run both server and serial connection ---
    esp_idf_svc::hal::task::block_on(async {
        let _ = futures::future::join(server_future, serial_future).await;
    });

    Ok(())
}

pub struct ServerRunData<'a> {
    veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>,
    veml_rgb: Option<Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    rgb_baseline: Option<(u16, u16, u16)>,
    dark_rgb_baseline: Option<(u16, u16, u16)>,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    ws2812b: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
    saved_rgb_multipliers: RGBMultipliers,
}

#[allow(clippy::too_many_arguments)]
pub async fn run<'a>(
    data: ServerRunData<'a>,
    stack: &edge_nal_std::Stack,
    server: &mut WsServer,
) -> Result<(), anyhow::Error> {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80));

    log::info!("Running HTTP server on {addr}");
    log::info!(
        "Loaded RGB multipliers: R={:.2}, G={:.2}, B={:.2}, Brightness={:.2}, TD_ref={:.2}",
        data.saved_rgb_multipliers.red,
        data.saved_rgb_multipliers.green,
        data.saved_rgb_multipliers.blue,
        data.saved_rgb_multipliers.brightness,
        data.saved_rgb_multipliers.td_reference
    );

    let acceptor = stack.bind(addr).await?;
    let ws_rgb_data = match data.veml_rgb {
        Some(some_veml_rgb) => Some(RgbWsHandler {
            dark_rgb_baseline: data.dark_rgb_baseline.unwrap(),
            rgb_baseline: data.rgb_baseline.unwrap(),
            rgb_buffers: Arc::new(Mutex::new((
                median_buffer::RunningMedianBufferU16::new(100),
                median_buffer::RunningMedianBufferU16::new(100),
                median_buffer::RunningMedianBufferU16::new(100),
            ))),
            veml_rgb: some_veml_rgb,
        }),
        None => None,
    };

    let handler = WsHandler {
        veml: data.veml,
        dark_baseline_reading: data.dark_baseline_reading,
        baseline_reading: data.baseline_reading,
        wifi_status: data.wifi_status,
        led_light: data.led_light,
        nvs: data.nvs,
        ws2812b: data.ws2812b,
        saved_algorithm: data.saved_algorithm,
        // Use smaller buffers to reduce memory usage
        lux_buffer: Arc::new(Mutex::new(median_buffer::RunningMedianBuffer::new(100))),
        rgb: ws_rgb_data,
        saved_rgb_multipliers: Arc::new(Mutex::new(data.saved_rgb_multipliers)),
    };

    match server.run(None, acceptor, handler).await {
        Ok(_) => (),
        Err(e) => log::error!("server.run: {e:?}"),
    };

    Ok(())
}

#[derive(Debug)]
enum WsHandlerError<C> {
    Connection(C),
}

impl<C> From<C> for WsHandlerError<C> {
    fn from(e: C) -> Self {
        Self::Connection(e)
    }
}

// Reduce the size of the future by using max 2 handler instead of 4
// and by limiting the number of headers to 32 instead of 64
type WsServer = Server<2, 1024, 16>; // Reduced from DEFAULT_BUF_SIZE and 32 headers

struct WsHandler<'a> {
    veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    ws2812b: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
    // Add median buffers
    lux_buffer: Arc<Mutex<median_buffer::RunningMedianBuffer>>,
    rgb: Option<RgbWsHandler>,
    saved_rgb_multipliers: Arc<Mutex<RGBMultipliers>>,
}
#[derive(Clone)]
pub struct RgbWsHandler {
    pub rgb_baseline: (u16, u16, u16),
    pub dark_rgb_baseline: (u16, u16, u16),
    pub veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>,
    pub rgb_buffers: Arc<
        Mutex<(
            median_buffer::RunningMedianBufferU16,
            median_buffer::RunningMedianBufferU16,
            median_buffer::RunningMedianBufferU16,
        )>,
    >,
}
