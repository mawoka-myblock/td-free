#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::net::Ipv4Addr;

use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, Runner, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::ledc::channel::ChannelIFace;
use esp_hal::ledc::timer::TimerIFace;
use esp_hal::ledc::{channel, timer, Ledc, LowSpeed};
use esp_hal::rmt::Rmt;
use esp_hal::rng::Rng;
use esp_hal::time::Rate;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::{smart_led_buffer, SmartLedsAdapter};
use esp_println::println;
use esp_wifi::wifi::{AccessPointConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiState};
use esp_wifi::{init, EspWifiController};
use log::info;
use smart_leds::{SmartLedsWrite, RGB};
use static_cell::make_static;

mod helpers;

extern crate alloc;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.3.1

    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals: esp_hal::peripherals::Peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    info!("Embassy initialized!");

    /* Starting LEDC Setup Block */

    let ledc = Ledc::new(peripherals.LEDC);
    let mut lstimer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    lstimer0
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty5Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(24),
        })
        .unwrap();
    let mut channel0: channel::Channel<'_, LowSpeed> =
        ledc.channel(channel::Number::Channel0, peripherals.GPIO7);
    channel0
        .configure(channel::config::Config {
            timer: &lstimer0,
            duty_pct: 10,
            pin_config: channel::config::PinConfig::PushPull,
        })
        .unwrap();

    /* Ending LEDC Setup Block */

    /* Starting Ws2812B Setup Block */
    let rmt: Rmt<'_, esp_hal::Blocking> = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let rmt_buffer_old = smart_led_buffer!(1);
    let mut old_led = SmartLedsAdapter::new(rmt.channel0, peripherals.GPIO21, rmt_buffer_old);
    let rmt_buffer_new = smart_led_buffer!(1);
    let mut new_led = SmartLedsAdapter::new(rmt.channel1, peripherals.GPIO4, rmt_buffer_new);
    old_led
        .write([RGB {
            r: 255,
            g: 255,
            b: 0,
        }])
        .unwrap();
    new_led
        .write([RGB {
            r: 255,
            g: 255,
            b: 0,
        }])
        .unwrap();
    /* Ending Ws2812B Setup Block */

    /* Starting Veml Setup */
    let mut i2c: I2c<'_, esp_hal::Blocking> = I2c::new(peripherals.I2C0, Config::default())
        .unwrap()
        .with_sda(peripherals.GPIO6)
        .with_scl(peripherals.GPIO5);

    /* Ending Veml Setup */
    /* Starting Wifi Setup */
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut rng = Rng::new(peripherals.RNG);
    let esp_wifi_ctrl =
        &*make_static!(init(timg0.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap());
    let (controller, interface) = esp_wifi::wifi::new(&esp_wifi_ctrl, peripherals.WIFI).unwrap();
    let device = interface.ap;
    let gw_ip_addr = Ipv4Addr::new(192, 168, 2, 1);
    let wifi_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr, 24),
        gateway: Some(gw_ip_addr),
        dns_servers: Default::default(),
    });
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        device,
        wifi_config,
        make_static!(StackResources::<3>::new()),
        seed,
    );
    /* Ending Wifi Setup */

    // TODO: Spawn some tasks
    let _ = spawner;

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(1)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-beta.0/examples/src/bin
}


#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.capabilities());
    loop {
        match esp_wifi::wifi::wifi_state() {
            WifiState::ApStarted => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::ApStop).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::AccessPoint(AccessPointConfiguration {
                ssid: "esp-wifi".try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start_async().await.unwrap();
            println!("Wifi started!");
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}