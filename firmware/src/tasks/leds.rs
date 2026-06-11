use embassy_time::Timer;
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
use smart_leds::{RGB8, SmartLedsWriteAsync};

#[embassy_executor::task]
pub async fn rgb_led_task(rmt_per: RMT<'static>, p4: GPIO4<'static>) {
    let mut rmt_buffer = [esp_hal::rmt::PulseCode::default(); buffer_size_async(1)];
    let mut led = {
        let freq = Rate::from_mhz(80);
        let rmt = Rmt::new(rmt_per, freq).unwrap().into_async();
        SmartLedsAdapterAsync::new(rmt.channel0, p4, &mut rmt_buffer)
    };
    led.write([RGB8::new(255, 255, 0)]).await.unwrap();
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
    Timer::after_secs(5).await;
    l.set_duty(20).unwrap();
}
