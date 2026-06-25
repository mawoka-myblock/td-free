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
use defmt::warn;
use embassy_executor::Spawner;
use embassy_futures::select::Either;
use embassy_futures::select::select;
use embassy_net::Ipv4Cidr;
use embassy_net::StaticConfigV4;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::system::software_reset;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use esp_println as _;
use esp_radio::wifi::ControllerConfig;
use esp_radio::wifi::ap::AccessPointConfig;
use esp_radio::wifi::sta::StationConfig;
use firmware::CLIENT_CONNECTED;
use firmware::helpers::storage::NvsStored as _;
use firmware::helpers::storage::WifiCreds;
use firmware::helpers::storage::nvs::Nvs;
use firmware::{NvsMutex, tasks};
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
    CLIENT_CONNECTED.sender().send(0);

    let nvs: &'static NvsMutex = firmware::mk_static!(
        NvsMutex,
        Mutex::new(Nvs::new(firmware::NVS_OFFSET, firmware::NVS_SIZE, peripherals.FLASH).unwrap())
    );

    let wifi_creds = tasks::states::init_signals_and_get_wifi_creds(nvs).await;

    spawner.spawn(tasks::states::data_update_save_task(nvs).unwrap());
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

    let (rx, tx) = UsbSerialJtag::new(peripherals.USB_DEVICE)
        .into_async()
        .split();
    spawner.spawn(tasks::serial::handle_serial_task(tx, rx).unwrap());

    let (iface, net_config, wifi_controller) = if let Some(wf_creds) = wifi_creds.clone() {
        info!("Found wifi creds, trying to connect");
        firmware::WIFI_STATE
            .sender()
            .send(firmware::WifiState::Connecting);
        let sta_config = esp_radio::wifi::Config::Station(
            StationConfig::default()
                .with_ssid(wf_creds.ssid.as_str())
                .with_password(wf_creds.password.as_str().into()),
        );
        let (mut wifi_controller, interfaces) = esp_radio::wifi::new(
            peripherals.WIFI,
            ControllerConfig::default().with_initial_config(sta_config),
        )
        .expect("Failed to initialize Wi-Fi controller");

        let connect_is_err = wifi_controller.connect_async().await.is_err();
        if connect_is_err {
            warn!("Resetting wifi creds");
            let _ = WifiCreds::delete(nvs).await;
            Timer::after_millis(200).await;
            software_reset();
        }

        let mut dhcp_cfg = embassy_net::DhcpConfig::default();

        let mut hostname = heapless::String::new();
        hostname.push_str("td-free").unwrap();
        dhcp_cfg.hostname = Some(hostname);
        (
            interfaces.station,
            embassy_net::Config::dhcpv4(dhcp_cfg),
            wifi_controller,
        )
    } else {
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
        firmware::WIFI_STATE
            .sender()
            .send(firmware::WifiState::HotSpotRunning);
        (
            interfaces.access_point,
            embassy_net::Config::ipv4_static(ipv4_cfg),
            wifi_controller,
        )
    };
    let (stack, runner) = embassy_net::new(
        iface,
        net_config,
        firmware::mk_static!(
            embassy_net::StackResources<7>,
            embassy_net::StackResources::<7>::new()
        ),
        make_seed(),
    );
    spawner.spawn(tasks::http::network::net_task(runner).unwrap());

    if wifi_creds.is_some() {
        let has_ip_fut = async {
            info!("Checking for ip addr");
            loop {
                if let Some(config) = stack.config_v4() {
                    info!("Got IP: {}", config.address);
                    return config.address;
                }
                Timer::after_millis(200).await;
            }
        };
        let timeout_fut = Timer::after_secs(600);
        match select(has_ip_fut, timeout_fut).await {
            Either::First(_) => (),
            Either::Second(_) => {
                warn!("Resetting wifi creds");
                let _ = WifiCreds::delete(nvs).await;
                Timer::after_millis(200).await;
                software_reset()
            }
        }
        firmware::WIFI_STATE
            .sender()
            .send(firmware::WifiState::Connected);
    } else {
        spawner.spawn(
            tasks::http::network::listen_for_connect_event_wifi_ap(wifi_controller).unwrap(),
        );
        spawner.spawn(tasks::http::network::run_dhcp_server(stack).unwrap());
        spawner.spawn(tasks::http::network::captive_dns(stack).unwrap());
    }

    let app = firmware::mk_static!(
        picoserve::AppRouter<tasks::http::AppProps>,
        tasks::http::AppProps::new().build_app()
    );

    for task_id in 0..tasks::http::WEB_TASK_POOL_SIZE {
        spawner.spawn(tasks::http::web_task(task_id, stack, app).unwrap());
    }

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

fn make_seed() -> u64 {
    let rng = esp_hal::rng::Rng::new();
    ((rng.random() as u64) << 32) | rng.random() as u64
}
