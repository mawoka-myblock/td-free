use defmt::info;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    pubsub::{PubSubBehavior, PubSubChannel},
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{RGB8, SmartLedsWriteAsync, brightness, gamma};

use crate::{
    CLIENT_CONNECTED, MEASUREMENT_STATE, MeasurementState, SETTINGS_DATA_WATCH, WIFI_STATE,
    WifiState,
};

pub type LedAdapter = SmartLedsAdapterAsync<'static, { buffer_size_async(1) }>;

static MEASURE_LED_CHANNEL: PubSubChannel<CriticalSectionRawMutex, MeasureLedCommand, 8, 1, 1> =
    PubSubChannel::new();

/// Command for the WS2812B measurement LED (GPIO7) sitting over the sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum MeasureLedCommand {
    Off,
    Rgb(u8, u8, u8),
}

impl MeasureLedCommand {
    pub fn white(level: u8) -> Self {
        Self::Rgb(level, level, level)
    }

    fn to_rgb8(self) -> RGB8 {
        match self {
            MeasureLedCommand::Off => RGB8::new(0, 0, 0),
            MeasureLedCommand::Rgb(r, g, b) => RGB8::new(r, g, b),
        }
    }
}

/// Drive the measurement LED. Values are written linearly (no gamma)
/// so illumination is proportional to the commanded levels.
pub fn set_measure_led(cmd: MeasureLedCommand) {
    MEASURE_LED_CHANNEL.publish_immediate(cmd);
}

#[embassy_executor::task]
pub async fn status_led_task(mut led: LedAdapter) {
    let mut settings_sub = SETTINGS_DATA_WATCH.receiver().unwrap();
    let mut measurement_sub = MEASUREMENT_STATE.receiver().unwrap();
    let mut wifi_sub = WIFI_STATE.receiver().unwrap();
    let mut client_sub = CLIENT_CONNECTED.receiver().unwrap();
    let mut brightness_pct = settings_sub.get().await.led_brightness;
    let mut measurement = measurement_sub.get().await;
    let mut wifi = wifi_sub.get().await;
    let mut client = client_sub.get().await;
    led.write(brightness(
        [RGB8::new(255, 255, 0)].into_iter(),
        brightness_pct,
    ))
    .await
    .unwrap();

    info!("Status LED task running");
    loop {
        info!("Update!, Brightness: {}", brightness_pct);
        let color = resolve_color(wifi, measurement, client);
        let brightness_val = (brightness_pct as u32 * 255 / 100) as u8;
        info!("Setting led: R: {} G: {}, B: {}", color.r, color.g, color.b);
        led.write(gamma(brightness([color].into_iter(), brightness_val)))
            .await
            .unwrap();
        embassy_futures::select::select4(
            async { brightness_pct = settings_sub.changed().await.led_brightness },
            async { measurement = measurement_sub.changed().await },
            async { wifi = wifi_sub.changed().await },
            async { client = client_sub.changed().await },
        )
        .await;
    }
}

fn resolve_color(wifi: WifiState, measurement: MeasurementState, client: bool) -> RGB8 {
    match (wifi, measurement) {
        (_, MeasurementState::Warmup) => RGB8::new(255, 255, 0),

        // HotSpot mode — show measurement state + client presence
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

#[embassy_executor::task]
pub async fn measure_led_task(mut led: LedAdapter) {
    let mut sub = MEASURE_LED_CHANNEL.subscriber().unwrap();
    led.write([RGB8::new(0, 0, 0)].into_iter()).await.unwrap();
    info!("Measurement LED task running");
    loop {
        let cmd = sub.next_message_pure().await;
        led.write([cmd.to_rgb8()].into_iter()).await.unwrap();
    }
}
