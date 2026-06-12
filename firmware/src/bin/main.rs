#![no_std]
#![recursion_limit = "512"]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use defmt::info;
use embassy_executor::Spawner;
use embassy_net::Ipv4Cidr;
use embassy_net::StaticConfigV4;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use esp_radio::wifi::ControllerConfig;
use esp_radio::wifi::ap::AccessPointConfig;
use firmware::CLIENT_CONNECTED;
use firmware::tasks;
use picoserve::AppBuilder;

extern crate alloc;

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
    CLIENT_CONNECTED.sender().send(false);

    spawner.spawn(tasks::leds::main_led_task(peripherals.LEDC, peripherals.GPIO7).unwrap());
    spawner.spawn(tasks::leds::rgb_led_task(peripherals.RMT, peripherals.GPIO4).unwrap());

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
            embassy_net::StackResources<7>,
            embassy_net::StackResources::<7>::new()
        ),
        seed,
    );

    let app = firmware::mk_static!(
        picoserve::AppRouter<tasks::http::AppProps>,
        tasks::http::AppProps::new().build_app()
    );

    spawner.spawn(tasks::http::network::listen_for_connect_event_wifi_ap(wifi_controller).unwrap());
    spawner.spawn(tasks::http::network::net_task(runner).unwrap());
    spawner.spawn(tasks::http::network::run_dhcp_server(stack).unwrap());
    spawner.spawn(tasks::http::network::captive_dns(stack).unwrap());

    for task_id in 0..tasks::http::WEB_TASK_POOL_SIZE {
        spawner.spawn(tasks::http::web_task(task_id, stack, app).unwrap());
    }

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.1.0/examples
}
