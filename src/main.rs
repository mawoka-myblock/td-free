#![feature(iter_intersperse)]
#![allow(clippy::await_holding_lock)]

use crate::helpers::baseline_readings::take_baseline_reading;
use crate::helpers::baseline_readings::take_rgb_white_balance_calibration;
use crate::helpers::bluetooth::init_bt;
use crate::helpers::bluetooth::server::RunData;
use crate::helpers::i2c_init::Pins;
use crate::helpers::i2c_init::initialize_veml;
use crate::helpers::median_buffer;
use crate::helpers::median_buffer::RunningMedianBuffer;
use crate::helpers::median_buffer::RunningMedianBufferU16;
use crate::helpers::nvs::NvsData;
use crate::helpers::nvs::RGBMultipliers;
use crate::helpers::nvs::clear_rgb_multipliers_nvs;
use crate::helpers::nvs::get_saved_algorithm_variables;
use crate::helpers::nvs::get_saved_rgb_multipliers;
use crate::helpers::readings::data_loop;
use crate::helpers::serial::serial_connection;
use core::fmt::Debug;
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use std::str;
use std::sync::{Arc, Mutex};

use edge_http::io::Error as EdgeError;
use edge_http::io::server::Server;
use edge_nal::TcpBind;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;

use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::usb_serial::{UsbSerialConfig, UsbSerialDriver};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvsPartition, NvsDefault};
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::{delay::FreeRtos, peripherals::Peripherals, prelude::*},
};

use helpers::bitbang_i2c::SimpleBitBangI2cInstance;
use log::info;
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::LedPixelEsp32Rmt;
use ws2812_esp32_rmt_driver::driver::color::LedPixelColorGrb24;

mod helpers;
mod led;
// mod routes;
mod veml3328;

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
    let veml_res = esp_idf_svc::hal::task::block_on(initialize_veml(
        Pins {
            i2c: peripherals.i2c0,
            sda1: peripherals.pins.gpio6,
            scl1: peripherals.pins.gpio5,
            sda2: peripherals.pins.gpio8,
            scl2: peripherals.pins.gpio10,
        },
        ws2812_old.clone(),
        ws2812_new.clone(),
    ));
    info!(
        "Old PCB? {}, Color? {}",
        veml_res.is_old_pcb,
        veml_res.veml3328.is_some()
    );
    let ws2812 = match veml_res.is_old_pcb {
        true => ws2812_old,
        false => ws2812_new,
    };

    let nvs = EspDefaultNvsPartition::take().unwrap();
    let nvs_clone = nvs.clone();

    // let veml: Arc<Mutex<VEML3328<I2cDriver<'_>>>> = Arc::new(Mutex::new(VEML3328::new(i2c)));
    led_light.lock().unwrap().set_duty_cycle_fully_on().unwrap();
    FreeRtos.delay_ms(500);
    let baseline_reading: f32 =
        esp_idf_svc::hal::task::block_on(take_baseline_reading(veml_res.veml7700.clone()));

    // White balance calibration at 50% LED brightness
    let rgb_white_balance: Option<(u16, u16, u16)> = match veml_res.veml3328.clone() {
        Some(d) => Some(esp_idf_svc::hal::task::block_on(
            take_rgb_white_balance_calibration(d.clone(), led_light.clone()),
        )),
        None => None,
    };

    led_light.lock().unwrap().set_duty(25).unwrap();
    FreeRtos.delay_ms(500);
    let dark_baseline_reading: f32 =
        esp_idf_svc::hal::task::block_on(take_baseline_reading(veml_res.veml7700.clone()));

    // For compatibility, we'll use the white balance as both baseline values
    let dark_rgb_baseline = rgb_white_balance;

    log::info!("Baseline readings completed with white balance calibration");

    let arced_nvs = Arc::new(nvs.clone());

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

    let measurement_channel = Arc::new(Channel::<NoopRawMutex, Option<String>, 1>::new());

    log::info!("Server created");
    let lux_buffer = Arc::new(Mutex::new(median_buffer::RunningMedianBuffer::new(100)));
    let rgb_buffers = Arc::new(Mutex::new((
        median_buffer::RunningMedianBufferU16::new(100),
        median_buffer::RunningMedianBufferU16::new(100),
        median_buffer::RunningMedianBufferU16::new(100),
    )));
    let ws_rgb_data = match veml_res.veml3328.clone() {
        Some(some_veml_rgb) => Some(RgbWsHandler {
            dark_rgb_baseline: dark_rgb_baseline.unwrap(),
            rgb_baseline: rgb_white_balance.unwrap(),
            rgb_buffers: rgb_buffers.clone(),
            veml_rgb: some_veml_rgb,
        }),
        None => None,
    };
    let run_data = RunData {
        lux_buffer: lux_buffer.clone(),
        nvs: arced_nvs.clone(),
        rgb: ws_rgb_data.clone(),
        saved_rgb_multipliers: saved_rgb_multipliers.clone(),
    };
    let bt_future = init_bt(
        peripherals.modem,
        nvs_clone,
        run_data,
        measurement_channel.clone(),
    );

    // --- Serial connection setup ---
    let mut serial_driver = UsbSerialDriver::new(
        peripherals.usb_serial,
        peripherals.pins.gpio18,
        peripherals.pins.gpio19,
        &UsbSerialConfig::new(),
    )
    .unwrap();
    let mut exit_buffer = [0u8; 1];
    FreeRtos.delay_ms(500);
    serial_driver.read(&mut exit_buffer, 500).unwrap();
    let cloned_serial_led = ws2812.clone();
    let cloned_mes_channel = measurement_channel.clone();
    let serial_future = {
        async move {
            if exit_buffer.contains(&b'e') {
                drop(serial_driver);
                std::future::pending::<Result<(), anyhow::Error>>().await
            } else {
                serial_connection(&mut serial_driver, cloned_serial_led, cloned_mes_channel).await
            }
        }
    };

    let measurement_future = data_loop(
        veml_res.veml7700.clone(),
        dark_baseline_reading,
        baseline_reading,
        led_light,
        ws2812,
        saved_algorithm,
        lux_buffer,
        ws_rgb_data,
        saved_rgb_multipliers,
        measurement_channel.clone(),
    );
    info!("Startup completed");

    // --- Run both server and serial connection ---
    esp_idf_svc::hal::task::block_on(async {
        let _ = futures::future::join3(bt_future, serial_future, measurement_future).await;
    });

    Ok(())
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
