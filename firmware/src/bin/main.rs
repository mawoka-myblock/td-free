#![no_std]
#![feature(type_alias_impl_trait)]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use bt_hci::controller::ExternalController;
use defmt::info;
use embassy_executor::Spawner;
use embassy_net::IpCidr;
use embassy_net::Ipv4Cidr;
use embassy_net::StaticConfigV4;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::rmt::Rmt;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::SmartLedsAdapterAsync;
use esp_hal_smartled::buffer_size_async;
use esp_println as _;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::ControllerConfig;
use esp_radio::wifi::ap::AccessPointConfig;
use firmware::tasks;
use picoserve::AppBuilder;
use static_cell::make_static;
use trouble_host::prelude::*;

extern crate alloc;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.3.0
    // generator parameters: --chip esp32c3 -o esp32c3-wroom-02 -o unstable-hal -o alloc -o wifi -o embassy -o ble-trouble -o esp-backtrace -o defmt -o zed -o nightly-x86_64-unknown-linux-gnu

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // The following pins are used to bootstrap the chip. They are available
    // for use, but check the datasheet of the module for more information on them.
    // - GPIO2
    // - GPIO8
    // - GPIO9
    // These GPIO pins are in use by some feature of the module and should not be used.
    let _ = peripherals.GPIO11;
    let _ = peripherals.GPIO12;
    let _ = peripherals.GPIO13;
    let _ = peripherals.GPIO14;
    let _ = peripherals.GPIO15;
    let _ = peripherals.GPIO16;
    let _ = peripherals.GPIO17;

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Embassy initialized!");

    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    // let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    // let ble_controller = ExternalController::<_, 1>::new(transport);
    // let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
    //     HostResources::new();
    // let _stack = trouble_host::new(ble_controller, &mut resources);

    spawner.spawn(tasks::leds::rgb_led_task(peripherals.RMT, peripherals.GPIO4).unwrap());
    spawner.spawn(tasks::leds::main_led_task(peripherals.LEDC, peripherals.GPIO7).unwrap());

    spawner.spawn(
        tasks::sensors::sensor_task(
            peripherals.GPIO6,
            peripherals.GPIO5,
            peripherals.GPIO8,
            peripherals.GPIO10,
            peripherals.I2C0,
        )
        .unwrap(),
    );
    let ap_config = esp_radio::wifi::Config::AccessPoint(
        AccessPointConfig::default()
            .with_ssid("Td-Free")
            .with_auth_method(esp_radio::wifi::AuthenticationMethod::None),
    );
    let (wifi_controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(ap_config),
    )
    .expect("Failed to initialize Wi-Fi controller");
    let ipv4_cfg = StaticConfigV4 {
        address: Ipv4Cidr::new(embassy_net::Ipv4Address::new(10, 10, 10, 1), 24),
        dns_servers: Default::default(),
        gateway: Some(embassy_net::Ipv4Address::new(10, 10, 10, 1)),
    };
    let config = embassy_net::Config::ipv4_static(ipv4_cfg);
    let rng = esp_hal::rng::Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let (stack, runner) = embassy_net::new(
        interfaces.access_point,
        config,
        firmware::mk_static!(
            embassy_net::StackResources<5>,
            embassy_net::StackResources::<5>::new()
        ),
        seed,
    );

    let app = firmware::mk_static!(
        picoserve::AppRouter<tasks::http::AppProps>,
        tasks::http::AppProps::new().build_app()
    );

    spawner.spawn(tasks::http::connection(wifi_controller).unwrap());
    spawner.spawn(tasks::http::net_task(runner).unwrap());
    spawner.spawn(tasks::http::run_dhcp(stack).unwrap());
    spawner.spawn(tasks::http::captive_dns(stack).unwrap());

    for task_id in 0..tasks::http::WEB_TASK_POOL_SIZE {
        spawner.spawn(tasks::http::web_task(task_id, stack, app).unwrap());
    }

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(1)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.1.0/examples
}
