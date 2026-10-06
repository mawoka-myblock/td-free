use defmt::info;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    pubsub::{PubSubBehavior, PubSubChannel},
};
use embedded_hal::pwm::SetDutyCycle;
use esp_hal::{
    ledc::{
        Ledc, LowSpeed,
        channel::{self, ChannelIFace},
        timer::{self, TimerIFace},
    },
    peripherals::{GPIO4, GPIO7, LEDC, RMT},
    rmt::Rmt,
    time::Rate,
};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{RGB8, SmartLedsWriteAsync, brightness, gamma};

use crate::{
    CLIENT_CONNECTED, MEASUREMENT_STATE, MeasurementState, SETTINGS_DATA_WATCH, WIFI_STATE,
    WifiState,
};

static LED_COMMAND_CHANNEL: PubSubChannel<CriticalSectionRawMutex, u8, 2, 1, 1> =
    PubSubChannel::new();

/// brightness in %
pub fn set_led_brightness(brightness: u8) {
    LED_COMMAND_CHANNEL.publish_immediate(brightness);
}

#[embassy_executor::task]
pub async fn rgb_led_task(rmt_per: RMT<'static>, p4: GPIO4<'static>) {
    let mut rmt_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];
    let mut led = {
        let freq = Rate::from_mhz(80);
        let rmt = Rmt::new(rmt_per, freq).unwrap().into_async();
        SmartLedsAdapterAsync::new(rmt.channel0, p4, &mut rmt_buffer)
    };
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

    info!("Now in loop");
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
pub async fn main_led_task(ledc_per: LEDC<'static>, p7: GPIO7<'static>) {
    let mut ledc = Ledc::new(ledc_per);
    ledc.set_global_slow_clock(esp_hal::ledc::LSGlobalClkSource::APBClk);
    let mut lstimer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    lstimer0
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty5Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(24),
        })
        .unwrap();
    let mut l = ledc.channel::<LowSpeed>(channel::Number::Channel0, p7);
    l.configure(channel::config::Config {
        timer: &lstimer0,
        duty_pct: 0,
        drive_mode: esp_hal::gpio::DriveMode::PushPull,
    })
    .unwrap();
    let mut sub = LED_COMMAND_CHANNEL.subscriber().unwrap();
    info!("Listening LED");
    loop {
        let msg = sub.next_message_pure().await;
        l.set_duty_cycle_percent(msg).unwrap();
    }
}
