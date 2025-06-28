#![feature(iter_intersperse)]
#![feature(let_chains)]

use core::fmt::{Debug, Display};
use crate::helpers::NvsData;
use crate::helpers::RGBMultipliers;
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::str;

use edge_http::io::server::Connection;
use edge_http::io::server::{Handler, Server, DEFAULT_BUF_SIZE};
use edge_http::io::Error as EdgeError;
use edge_http::Method as EdgeMethod;
use edge_nal::TcpBind;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;

use embedded_hal::delay::DelayNs;
use embedded_hal::pwm::SetDutyCycle;

use embedded_io_async::{Read, Write};

use esp_idf_svc::hal::ledc::config::TimerConfig;
use esp_idf_svc::hal::ledc::{LedcDriver, LedcTimerDriver};
use esp_idf_svc::hal::task::block_on;
use esp_idf_svc::hal::usb_serial::UsbSerialDriver;
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
    hal::{delay::FreeRtos, peripherals::Peripherals, prelude::*},
};

use helpers::{generate_random_11_digit_number, Pins, initialize_veml, HardwareI2cInstance, SimpleBitBangI2cInstance};
use led::set_led;
use smart_leds::RGB8;
use veml7700::Veml7700;
use wifi::WifiEnum;
use ws2812_esp32_rmt_driver::driver::color::LedPixelColorGrb24;
use ws2812_esp32_rmt_driver::LedPixelEsp32Rmt;

mod helpers;
mod led;
mod median_buffer;
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

    let ws2812_old: Arc<
        Mutex<
            LedPixelEsp32Rmt<
                '_,
                smart_leds::RGB<u8>,
                ws2812_esp32_rmt_driver::driver::color::LedPixelColorImpl<3, 1, 0, 2, 255>,
            >,
        >,
    > = Arc::new(Mutex::new(
        LedType::new(peripherals.rmt.channel0, peripherals.pins.gpio21).unwrap(),
    ));
    let ws2812_new: Arc<
        Mutex<
            LedPixelEsp32Rmt<
                '_,
                smart_leds::RGB<u8>,
                ws2812_esp32_rmt_driver::driver::color::LedPixelColorImpl<3, 1, 0, 2, 255>,
            >,
        >,
    > = Arc::new(Mutex::new(
        LedType::new(peripherals.rmt.channel1, peripherals.pins.gpio4).unwrap(),
    ));
    ws2812_old.lock().unwrap().write_nocopy(std::iter::repeat(RGB8::new(255, 255, 0)).take(1)).unwrap();
    ws2812_new.lock().unwrap().write_nocopy(std::iter::repeat(RGB8::new(255, 255, 0)).take(1)).unwrap();
    let (veml, veml_rgb, is_old_pcb) = initialize_veml(
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
    let ws2812 = match is_old_pcb {
        true => ws2812_old,
        false => ws2812_new,
    };
    led_light.lock().unwrap().set_duty_cycle_fully_on().unwrap();
    // let i2c_scl_bb = PinDriver::output(peripherals.pins.gpio10).unwrap();
    // let i2c_sda_bb = PinDriver::input_output(peripherals.pins.gpio8).unwrap();
    // let timer_cfg = esp_idf_svc::hal::timer::config::Config{auto_reload: false, divider: 2, xtal: false};
    // let i2c_clk_bb = TimerDriver::new(peripherals.timer00, &timer_cfg).unwrap();
    // let i2c_color = bitbang_hal::i2c::I2cBB::new(i2c_scl_bb, i2c_sda_bb, i2c_clk_bb);
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

    // let veml: Arc<Mutex<VEML3328<I2cDriver<'_>>>> = Arc::new(Mutex::new(VEML3328::new(i2c)));
    FreeRtos.delay_ms(500);
    let baseline_reading: f32 = take_baseline_reading(veml.clone());

    // White balance calibration at 50% LED brightness
    let rgb_white_balance: (u16, u16, u16) = take_rgb_white_balance_calibration(veml_rgb.clone(), led_light.clone());

    led_light.lock().unwrap().set_duty(25).unwrap();
    FreeRtos.delay_ms(400);
    let dark_baseline_reading: f32 = take_baseline_reading(veml.clone());

    // For compatibility, we'll use the white balance as both baseline values
    let dark_rgb_baseline: (u16, u16, u16) = rgb_white_balance;

    log::info!("Baseline readings completed with white balance calibration");
    let wifi_status: Arc<Mutex<WifiEnum>> = Arc::new(Mutex::new(WifiEnum::Working));

    let _hotspot_ip = block_on(wifi::wifi_setup(
        &mut wifi,
        nvs.clone(),
        ws2812.clone(),
        wifi_status.clone(),
    ))
    .unwrap()
    .1;
    log::info!("WiFi Started");

    let arced_nvs = Arc::new(nvs.clone());

    let mut server = unsafe { Box::new_uninit().assume_init() };

    let _eventfd = esp_idf_svc::io::vfs::MountedEventfs::mount(3);
    
    // Try to load algorithm variables with error recovery
    let saved_algorithm = match std::panic::catch_unwind(|| {
        helpers::get_saved_algorithm_variables(arced_nvs.as_ref().clone())
    }) {
        Ok(algorithm) => algorithm,
        Err(_) => {
            log::error!("Algorithm loading caused panic - using defaults");
            helpers::NvsData {
                b: 0.0,
                m: 1.0,
                threshold: 0.8,
            }
        }
    };
    
    // Try to load RGB multipliers with error recovery
    let saved_rgb_multipliers = match std::panic::catch_unwind(|| {
        helpers::get_saved_rgb_multipliers(arced_nvs.as_ref().clone())
    }) {
        Ok(multipliers) => multipliers,
        Err(_) => {
            log::error!("RGB multipliers loading caused panic - clearing NVS and using defaults");
            // Clear the corrupted data
            if let Err(e) = helpers::clear_rgb_multipliers_nvs(arced_nvs.as_ref().clone()) {
                log::error!("Failed to clear RGB multipliers NVS: {:?}", e);
            }
            helpers::RGBMultipliers::default()
        }
    };

    log::info!("Server created");
    let stack = edge_nal_std::Stack::new();
    let server_future = run(
        &mut server,
        veml.clone(),
        veml_rgb.clone(),
        dark_baseline_reading,
        baseline_reading,
        rgb_white_balance,  // Use white balance instead of rgb_baseline
        dark_rgb_baseline,
        wifi_status.clone(),
        led_light.clone(),
        arced_nvs.clone(),
        &stack,
        ws2812.clone(),
        saved_algorithm,
        saved_rgb_multipliers,
    );

    // Block on the server future
    block_on(server_future).unwrap();

    Ok(())
}

fn take_rgb_white_balance_calibration(
    veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>,
    led_light: Arc<Mutex<LedcDriver<'_>>>
) -> (u16, u16, u16) {
    let sample_count = 20; // Increased samples for better accuracy
    let sample_delay = 50u32;

    log::info!("Starting comprehensive RGB white balance calibration with {} samples", sample_count);

    // Take calibration readings at multiple brightness levels to account for non-linearity
    let brightness_levels = [10, 25 ,50, 75]; // Different LED brightness levels
    let mut all_r_readings: Vec<u16> = Vec::new();
    let mut all_g_readings: Vec<u16> = Vec::new();
    let mut all_b_readings: Vec<u16> = Vec::new();
    let mut all_clear_readings: Vec<u16> = Vec::new();

    for &brightness in &brightness_levels {
        log::info!("Taking calibration readings at {}% brightness", brightness);

        {
            let mut led = led_light.lock().unwrap();
            led.set_duty(brightness).unwrap();
        }

        // Wait for LED to stabilize
        FreeRtos.delay_ms(300);

        let mut r_readings: Vec<u16> = Vec::new();
        let mut g_readings: Vec<u16> = Vec::new();
        let mut b_readings: Vec<u16> = Vec::new();
        let mut clear_readings: Vec<u16> = Vec::new();

        for i in 0..sample_count {
            let mut locked_veml = veml_rgb.lock().unwrap();
            match (locked_veml.read_red(), locked_veml.read_green(), locked_veml.read_blue(), locked_veml.read_clear()) {
                (Ok(r), Ok(g), Ok(b), Ok(clear)) => {
                    log::debug!("Brightness {}% sample {}: R={}, G={}, B={}, Clear={}", brightness, i, r, g, b, clear);
                    r_readings.push(r);
                    g_readings.push(g);
                    b_readings.push(b);
                    clear_readings.push(clear);
                },
                (r_result, g_result, b_result, clear_result) => {
                    log::warn!("Failed to read RGB sensor - R: {:?}, G: {:?}, B: {:?}, Clear: {:?}",
                              r_result, g_result, b_result, clear_result);
                    continue;
                }
            }
            drop(locked_veml);
            FreeRtos.delay_ms(sample_delay);
        }

        // Add readings from this brightness level to overall collection
        all_r_readings.extend(r_readings);
        all_g_readings.extend(g_readings);
        all_b_readings.extend(b_readings);
        all_clear_readings.extend(clear_readings);
    }

    if all_r_readings.is_empty() {
        log::error!("No valid RGB readings obtained during white balance, using default values");
        return (1000, 1000, 1000);
    }

    // Calculate median values across all brightness levels
    all_r_readings.sort();
    all_g_readings.sort();
    all_b_readings.sort();
    all_clear_readings.sort();

    let r_median = all_r_readings[all_r_readings.len() / 2];
    let g_median = all_g_readings[all_g_readings.len() / 2];
    let b_median = all_b_readings[all_b_readings.len() / 2];
    let clear_median = if !all_clear_readings.is_empty() {
        all_clear_readings[all_clear_readings.len() / 2]
    } else {
        r_median + g_median + b_median
    };

    // Calculate spectral response ratios for proper white balance
    // Assume green channel as reference (typically most sensitive in visible range)
    let g_ref = g_median as f32;
    let r_ratio = r_median as f32 / g_ref;
    let b_ratio = b_median as f32 / g_ref;

    // Apply color temperature correction based on my measurements for the sensor
    let led_color_temp_correction_r = 1.00;
    let led_color_temp_correction_g = 1.00;
    let led_color_temp_correction_b = 1.00;


    let corrected_r = (r_median as f32 * led_color_temp_correction_r) as u16;
    let corrected_g = (g_median as f32 * led_color_temp_correction_g) as u16;
    let corrected_b = (b_median as f32 * led_color_temp_correction_b) as u16;

    log::info!("RGB white balance raw medians: R={}, G={}, B={}, Clear={}",
              r_median, g_median, b_median, clear_median);
    log::info!("Spectral response ratios (relative to Green): R={:.3}, B={:.3}", r_ratio, b_ratio);
    log::info!("Color temperature corrected: R={}, G={}, B={}",
              corrected_r, corrected_g, corrected_b);

    // Return color temperature corrected values
    (corrected_r, corrected_g, corrected_b)
}

pub async fn serial_connection<'a>(
    conn: &mut UsbSerialDriver<'static>,
    veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>,
    veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    rgb_baseline: (u16, u16, u16),
    dark_rgb_baseline: (u16, u16, u16),
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    ws2812: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
) -> Result<(), anyhow::Error> {
    let mut buffer = [0u8; 64]; // Buffer for reading incoming data
    let trigger_measurement = Arc::new(AtomicBool::new(false));
    let trigger_clone = trigger_measurement.clone();
    let channel = Channel::<NoopRawMutex, String, 1>::new();
    let recv = channel.receiver();
    let send = channel.sender();

    // Create median buffers for serial connection
    let lux_buffer = Arc::new(Mutex::new(median_buffer::RunningMedianBuffer::new(100)));
    let rgb_buffers = Arc::new(Mutex::new((
        median_buffer::RunningMedianBufferU16::new(100),
        median_buffer::RunningMedianBufferU16::new(100),
        median_buffer::RunningMedianBufferU16::new(100),
    )));

    // Create default RGB multipliers for serial connection
    let rgb_multipliers = Arc::new(Mutex::new(RGBMultipliers::default()));

    let conn_loop = async {
        loop {
            // Step 1: Wait for "connect\n" from Python script
            if trigger_measurement.load(Ordering::SeqCst) {
                embassy_time::Timer::after_millis(300).await;
            } else {
                embassy_time::Timer::after_millis(600).await;
            }

            let n = match conn.read(&mut buffer, 50) {
                Ok(n) if n > 0 => Some(n),
                _ => None, // No data, continue looping
            };

            if let Some(n) = n {
                let received: &str = core::str::from_utf8(&buffer[..n]).unwrap_or("").trim();

                match received {
                    "connect" => {
                        conn.write(b"ready\n", 100).unwrap();
                    }
                    "P" | "HF" => {
                        trigger_measurement.store(true, Ordering::SeqCst);
                        conn.write(b"connected to HF unlicensed\n", 100).unwrap();
                    }
                    "version" => {
                        conn.write(b"result, TD1 Version: V1.0.4, StatusScreen Version: V1.0.4,Comms Version: V1.0.4, startUp Version: V1.0.4\n", 100).unwrap();
                    }
                    _ => {}
                }
                conn.flush().unwrap();
            } else if trigger_measurement.load(Ordering::SeqCst)
                && let Ok(msg) = recv.try_receive()
            {
                conn.write(msg.as_bytes(), 500).unwrap();
                conn.flush().unwrap();
                recv.clear();
            }
            continue;
        }
    };
    let measurement_loop = async {
        loop {
            if !trigger_clone.load(Ordering::SeqCst) {
                embassy_time::Timer::after_millis(500).await;
                continue;
            }
            set_led(ws2812.clone(), 100, 30, 255);

            // Use fast current reading for filament detection
            let is_filament_inserted = {
                let mut locked_veml = veml.lock().unwrap();
                match locked_veml.read_lux() {
                    Ok(reading) => {
                        let current_reading = reading as f32;
                        current_reading / dark_baseline_reading <= saved_algorithm.threshold
                    },
                    Err(_) => false,
                }
            };

            if !is_filament_inserted {
                embassy_time::Timer::after_millis(300).await;
                continue;
            }
            embassy_time::Timer::after_millis(50).await;

            let reading = routes::read_averaged_data_with_buffer(
                veml.clone(),
                veml_rgb.clone(),
                dark_baseline_reading,
                baseline_reading,
                rgb_baseline,
                dark_rgb_baseline,
                wifi_status.clone(),
                led_light.clone(),
                ws2812.clone(),
                saved_algorithm,
                lux_buffer.clone(),
                rgb_buffers.clone(),
                rgb_multipliers.clone(),
            )
            .await;
            
            set_led(ws2812.clone(), 255, 30, 255);
            let reading_string = reading.unwrap();
            let reading_parts: Vec<&str> = reading_string.split(',').collect();
            let reading_float = reading_parts[0].parse::<f32>().unwrap();
            let message = format!(
                "{},,,,{:.1},000000\n",
                generate_random_11_digit_number(),
                reading_float
            );
            send.send(message).await;

            embassy_time::Timer::after_millis(300).await;
            loop {
                embassy_time::Timer::after_millis(50).await; // Much faster polling for filament removal

                // Use fast current reading for filament removal detection
                let is_filament_inserted = {
                    let mut locked_veml = veml.lock().unwrap();
                    match locked_veml.read_lux() {
                        Ok(reading) => {
                            let current_reading = reading as f32;
                            current_reading / dark_baseline_reading <= saved_algorithm.threshold
                        },
                        Err(_) => false,
                    }
                };

                if !is_filament_inserted {
                    break;
                }
            }

            embassy_time::Timer::after_millis(100).await;
        }
    };
    embassy_futures::join::join(measurement_loop, conn_loop).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run<'a>(
    server: &mut WsServer,
    veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>,
    veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    rgb_baseline: (u16, u16, u16),
    dark_rgb_baseline: (u16, u16, u16),
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    stack: &edge_nal_std::Stack,
    ws2812b: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
    saved_rgb_multipliers: RGBMultipliers,
) -> Result<(), anyhow::Error> {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 80));

    log::info!("Running HTTP server on {addr}");
    log::info!("Loaded RGB multipliers: R={:.2}, G={:.2}, B={:.2}, Brightness={:.2}, TD_ref={:.2}",
              saved_rgb_multipliers.red, saved_rgb_multipliers.green, saved_rgb_multipliers.blue, 
              saved_rgb_multipliers.brightness, saved_rgb_multipliers.td_reference);

    let acceptor = stack.bind(addr).await?;

    let handler = WsHandler {
        veml,
        veml_rgb,
        dark_baseline_reading,
        baseline_reading,
        rgb_baseline,
        dark_rgb_baseline,
        wifi_status,
        led_light,
        nvs,
        ws2812b,
        saved_algorithm,
        saved_rgb_multipliers: Arc::new(Mutex::new(saved_rgb_multipliers)),
        // Use smaller buffers to reduce memory usage
        lux_buffer: Arc::new(Mutex::new(median_buffer::RunningMedianBuffer::new(100))),
        rgb_buffers: Arc::new(Mutex::new((
            median_buffer::RunningMedianBufferU16::new(100),
            median_buffer::RunningMedianBufferU16::new(100),
            median_buffer::RunningMedianBufferU16::new(100),
        ))),
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
type WsServer = Server<2, 1024, 16>; // Reduced from DEFAULT_BUF_SIZE and 32 headers

struct WsHandler<'a> {
    veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>,
    veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>,
    dark_baseline_reading: f32,
    baseline_reading: f32,
    rgb_baseline: (u16, u16, u16),
    dark_rgb_baseline: (u16, u16, u16),
    wifi_status: Arc<Mutex<WifiEnum>>,
    led_light: Arc<Mutex<LedcDriver<'a>>>,
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    ws2812b: Arc<Mutex<LedType<'a>>>,
    saved_algorithm: NvsData,
    saved_rgb_multipliers: Arc<Mutex<RGBMultipliers>>,
    // Add median buffers
    lux_buffer: Arc<Mutex<median_buffer::RunningMedianBuffer>>,
    rgb_buffers: Arc<Mutex<(
        median_buffer::RunningMedianBufferU16,
        median_buffer::RunningMedianBufferU16,
        median_buffer::RunningMedianBufferU16,
    )>>,
}


fn serve_wifi_setup_page(current_ssid: &str, error: &str) -> String {
    format!(
        include_str!("wifi_setup.html"),
        ssid = current_ssid,
        error = error
    )
}

fn serve_algo_setup_page(b_val: f32, m_val: f32, threshold_val: f32, spoolman_val: &str, spoolman_field_name: &str) -> String {
    format!(
        include_str!("settings.html"),
        b_val = b_val,
        m_val = m_val,
        threshold_val = threshold_val,
        spoolman_val = spoolman_val,
        spoolman_field_name = spoolman_field_name
    )
}

fn take_baseline_reading(veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>>) -> f32 {
    let sample_count = 20;
    let sample_delay = 50u32;
    let mut readings: Vec<f32> = Vec::with_capacity(sample_count as usize);

    for _ in 0..sample_count {
        let mut locked_veml = veml.lock().unwrap();
        let clr = match locked_veml.read_lux() {
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
        let reading = clr as f32;
        log::info!("Reading: {}", reading);
        readings.push(reading);
        FreeRtos.delay_ms(sample_delay);
    }

    if readings.is_empty() {
        return 0.0; // Avoid divide by zero or panics later
    }

    // Calculate mean and std deviation
    let mean = readings.iter().copied().sum::<f32>() / readings.len() as f32;
    let std = (readings.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / readings.len() as f32).sqrt();

    // Filter out outliers
    let mut filtered: Vec<f32> = readings
    .into_iter()
    .filter(|v| (*v - mean).abs() <= 2.0 * std)
    .collect();

    // Calculate median from filtered data
    if filtered.is_empty() {
        return mean; // fallback to mean if all readings were filtered out
    }

    filtered.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if filtered.len() % 2 == 0 {
        let mid = filtered.len() / 2;
        (filtered[mid - 1] + filtered[mid]) / 2.0
    } else {
        filtered[filtered.len() / 2]
    };

    median
}
