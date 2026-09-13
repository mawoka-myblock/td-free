use defmt::info;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    pubsub::{PubSubBehavior, PubSubChannel},
};
use esp_hal::{
    peripherals::{GPIO4, GPIO7, RMT},
    rmt::Rmt,
    time::Rate,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{RGB8, SmartLedsWriteAsync, gamma};

use crate::{CLIENT_CONNECTED, MEASUREMENT_STATE, MeasurementState, WIFI_STATE, WifiState};

static LED_COMMAND_CHANNEL: PubSubChannel<CriticalSectionRawMutex, RGB8, 2, 1, 1> =
    PubSubChannel::new();

/// Sets the measurement LED to an arbitrary RGB colour.
pub fn set_led_color(color: RGB8) {
    LED_COMMAND_CHANNEL.publish_immediate(color);
}

/// Sets the measurement LED to a uniform white brightness (0-100 %).
pub fn set_led_brightness(brightness: u8) {
    let v = brightness.min(100);
    let meas_val = (v as u32 * 255 / 100).min(255) as u8;
    set_led_color(RGB8::new(meas_val, meas_val, meas_val));
}

/// Drives both WS2812B LEDs:
/// - GPIO7: measurement LED (RGB colour controlled by the sensor task)
/// - GPIO4: indicator/status LED (colour based on device state)
#[embassy_executor::task]
pub async fn led_task(rmt_per: RMT<'static>, p7: GPIO7<'static>, p4: GPIO4<'static>) {
    let freq = Rate::from_mhz(80);
    let rmt = Rmt::new(rmt_per, freq).unwrap().into_async();

    let mut meas_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];
    let mut ind_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];

    let mut measurement_led = SmartLedsAdapterAsync::new(rmt.channel0, p7, &mut meas_buffer);
    let mut indicator_led = SmartLedsAdapterAsync::new(rmt.channel1, p4, &mut ind_buffer);

    let mut brightness_sub = LED_COMMAND_CHANNEL.subscriber().unwrap();
    let mut measurement_sub = MEASUREMENT_STATE.receiver().unwrap();
    let mut wifi_sub = WIFI_STATE.receiver().unwrap();
    let mut client_sub = CLIENT_CONNECTED.receiver().unwrap();

    // Measurement LED starts off; indicator shows warm-up.
    measurement_led
        .write(gamma([RGB8::new(0, 0, 0)].into_iter()))
        .await
        .unwrap();
    indicator_led
        .write(gamma([RGB8::new(255, 255, 0)].into_iter()))
        .await
        .unwrap();

    let mut measurement_color = RGB8::new(0, 0, 0);
    let mut measurement = measurement_sub.get().await;
    let mut wifi = wifi_sub.get().await;
    let mut client = client_sub.get().await;

    info!("LED task ready");

    loop {
        // Measurement LED: driven directly by the commanded colour.
        measurement_led
            .write(gamma([measurement_color].into_iter()))
            .await
            .unwrap();

        // Indicator LED: status colour.
        let status_color = resolve_status_color(wifi, measurement, client);
        indicator_led
            .write(gamma([status_color].into_iter()))
            .await
            .unwrap();

        embassy_futures::select::select4(
            async { measurement_color = brightness_sub.next_message_pure().await },
            async { measurement = measurement_sub.changed().await },
            async { wifi = wifi_sub.changed().await },
            async { client = client_sub.changed().await },
        )
        .await;
    }
}

fn resolve_status_color(wifi: WifiState, measurement: MeasurementState, client: bool) -> RGB8 {
    match (wifi, measurement) {
        (_, MeasurementState::Warmup) => RGB8::new(255, 255, 0),

        // HotSpot mode
        (WifiState::HotSpotRunning, MeasurementState::FilamentInserted) => RGB8::new(0, 255, 255),
        (WifiState::HotSpotRunning, MeasurementState::Idle) if client => RGB8::new(255, 0, 255),
        (WifiState::HotSpotRunning, MeasurementState::Idle) => RGB8::new(180, 0, 180),

        // WiFi path
        (WifiState::Connecting, _) => RGB8::new(0, 0, 255),
        (WifiState::Connected, MeasurementState::FilamentInserted) => RGB8::new(0, 255, 255),
        (WifiState::Connected, MeasurementState::Idle) if client => RGB8::new(0, 255, 0),
        (WifiState::Connected, MeasurementState::Idle) => RGB8::new(0, 180, 0),
    }
}
