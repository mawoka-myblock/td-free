use defmt::{info, unwrap};
use embassy_time::Timer;
use esp_hal::{
    Blocking,
    i2c::master::{Config, I2c},
    peripherals,
    time::Rate,
};
use heapless::Vec;
use smart_leds::RGB8;

use crate::{
    CLIENT_CONNECTED, MEASUREMENT_DATA_WATCH, MEASUREMENT_STATE, MeasurementData, MeasurementState,
    SETTINGS_DATA_WATCH,
    helpers::{median_buffer::RunningMedianBuffer, v77::take_baseline_reading},
    tasks::{leds::{set_led_brightness, set_led_color}, states::init_dev_info},
};

pub type VEML7700<'d> = veml7700::Veml7700<I2c<'d, Blocking>>;

const TD_SCALE: f32 = 10.0;

/// Convert a transmission ratio (measured/reference, clipped at a small floor)
/// into a TD-1-style transmission density value: -log10(ratio) * scale.
fn transmission_to_td(ratio: f32) -> f32 {
    let safe_ratio = ratio.max(1e-4);
    -micromath::F32Ext::log10(safe_ratio) * TD_SCALE
}
const MEASUREMENT_COLOURS: [RGB8; 3] = [
    RGB8::new(255, 0, 0),
    RGB8::new(0, 255, 0),
    RGB8::new(0, 0, 255),
];

#[embassy_executor::task]
pub async fn sensor_task(
    _sda_v77: peripherals::GPIO6<'static>,
    _scl_v77: peripherals::GPIO5<'static>,
    _i2c0_per: peripherals::I2C0<'static>,
) {
    let dev_state_sender = MEASUREMENT_STATE.sender();
    dev_state_sender.send(MeasurementState::Warmup);

    let mut v77 = get_v77();
    v77.enable().unwrap();
    Timer::after_millis(200).await;

    // Bright-field baseline for transmission-density calculation.
    set_led_brightness(100);
    info!("LED at 100%");
    Timer::after_millis(150).await;
    let v77_baseline_bright = take_baseline_reading(&mut v77).await;

    // Dark-field baseline for filament-insertion detection.
    set_led_brightness(25);
    info!("LED at 25%");
    Timer::after_millis(150).await;
    let v77_baseline_dark = take_baseline_reading(&mut v77).await;

    // Capture per-channel references with no filament in the light path.
    // Drive the RGB LED one pure colour at a time and read VEML7700 lux.
    // These references are fixed and used for every measurement.
    let mut channel_references: [f32; 3] = [1.0; 3];
    for (i, &colour) in MEASUREMENT_COLOURS.iter().enumerate() {
        set_led_color(colour);
        info!("Channel {} reference LED on", i);
        Timer::after_millis(200).await;
        let mut v77 = get_v77();
        let mut readings: Vec<f32, 8> = Vec::new();
        for j in 0..3 {
            if j > 0 {
                Timer::after_millis(100).await;
            }
            if let Ok(lux) = v77.read_lux() {
                let _ = readings.push(lux);
            }
        }
        readings.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let lux = if readings.is_empty() { 0.0 } else { readings[readings.len() / 2] };
        channel_references[i] = lux.max(1.0);
        info!(
            "Channel {} reference lux={}",
            i, channel_references[i]
        );
    }

    set_led_brightness(0);

    init_dev_info(true).await;

    let mut lux_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();

    let mut client_connected_sub = unwrap!(CLIENT_CONNECTED.receiver());
    let mut settings_sub = SETTINGS_DATA_WATCH.anon_receiver();
    let mm_data_pub = MEASUREMENT_DATA_WATCH.sender();

    dev_state_sender.send(MeasurementState::Idle);

    loop {
        if !client_connected_sub.get().await {
            set_led_brightness(0);
            client_connected_sub.changed().await;
        }

        let saved_algorithm = settings_sub.try_get().expect("No Settings available").algo;

        let is_filament_inserted = {
            let mut v77 = get_v77();
            crate::helpers::v77::is_filament_inserted(
                &mut v77,
                v77_baseline_dark,
                saved_algorithm.threshold,
            )
            .await
            .0
        };

        if !is_filament_inserted {
            lux_buf.clear();
            dev_state_sender.send_if_modified(|v| {
                let changed = *v != Some(MeasurementState::Idle);
                *v = Some(MeasurementState::Idle);
                changed
            });
            mm_data_pub.send_if_modified(|v| {
                let changed = *v != Some(None);
                *v = Some(None);
                changed
            });
            continue;
        }

        info!("Filament detected");
        dev_state_sender.send_if_modified(|v| {
            let changed = *v != Some(MeasurementState::FilamentInserted);
            *v = Some(MeasurementState::FilamentInserted);
            changed
        });

        set_led_brightness(100);
        Timer::after_millis(300).await;

        let mut v77 = get_v77();
        let mut per_channel_td = [0.0_f32; 3];

        // Bright white reading for the overall TD.
        for i in 0..3 {
            if i > 0 {
                Timer::after_millis(100).await;
            }
            let lux = v77.read_lux().unwrap_or(0.0);
            info!("Raw lux: {}", lux);
            lux_buf.push(lux);
        }

        let buffer_count = lux_buf.len();
        let final_median_lux = lux_buf.median().unwrap_or(0.0);
        let white_ratio = (final_median_lux / v77_baseline_bright).clamp(1e-4, 1.0);
        let white_td = transmission_to_td(white_ratio);
        let adjusted_td = saved_algorithm.m * white_td + saved_algorithm.b;

        // Per-channel colour measurement using the RGB LED.
        for (i, &colour) in MEASUREMENT_COLOURS.iter().enumerate() {
            set_led_color(colour);
            Timer::after_millis(150).await;
            let sample_lux = v77.read_lux().unwrap_or(0.0);
            let ref_value = channel_references[i];
            info!("Channel {} sample lux: {}, ref={}", i, sample_lux, ref_value);

            let ratio = (sample_lux / ref_value).clamp(0.0, 1.0);
            per_channel_td[i] = transmission_to_td(ratio);
            info!(
                "Channel {}: measured={}, ref={}, ratio={}, td={}",
                i, sample_lux, ref_value, ratio, per_channel_td[i]
            );
        }

        let hex_color = relative_hex_from_tds(per_channel_td);

        info!(
            "td={}, per-channel td=[{}, {}, {}], white_ratio={}, median_lux={}",
            adjusted_td,
            per_channel_td[0],
            per_channel_td[1],
            per_channel_td[2],
            white_ratio,
            final_median_lux
        );

        mm_data_pub.send(Some(MeasurementData {
            td: adjusted_td,
            td_r: per_channel_td[0],
            td_g: per_channel_td[1],
            td_b: per_channel_td[2],
            buf_count: Some(buffer_count as u32),
            hex_color,
        }));
    }
}

fn relative_hex_from_tds(tds: [f32; 3]) -> Option<heapless::String<6>> {
    use core::fmt::Write;
    let max_td = tds.iter().copied().fold(0.0_f32, |a, b| a.max(b)).max(0.1);
    let scale = 255.0 / max_td;
    let rr = (tds[0] * scale).clamp(0.0, 255.0) as u8;
    let gg = (tds[1] * scale).clamp(0.0, 255.0) as u8;
    let bb = (tds[2] * scale).clamp(0.0, 255.0) as u8;
    let mut hex: heapless::String<6> = heapless::String::new();
    let _ = write!(hex, "{:02X}{:02X}{:02X}", rr, gg, bb);
    Some(hex)
}

fn get_i2c<'d>() -> I2c<'d, Blocking> {
    I2c::new(
        unsafe { peripherals::I2C0::steal() },
        Config::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
    .with_sda(unsafe { peripherals::GPIO6::steal() })
    .with_scl(unsafe { peripherals::GPIO5::steal() })
}

fn get_v77<'d>() -> VEML7700<'d> {
    veml7700::Veml7700::new(get_i2c())
}
