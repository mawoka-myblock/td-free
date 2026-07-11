use defmt::{debug, info, unwrap};
use embassy_time::Timer;
use esp_hal::{
    Blocking,
    i2c::master::{Config, I2c},
    peripherals,
    time::Rate,
};

use crate::{
    CLIENT_CONNECTED, MEASUREMENT_DATA_WATCH, MEASUREMENT_STATE, MeasurementData,
    MeasurementState, SETTINGS_DATA_WATCH,
    helpers::{median_buffer::RunningMedianBuffer, v77::take_baseline_reading},
    tasks::{leds::set_led_brightness, states::init_dev_info},
};

pub type VEML7700<'d> = veml7700::Veml7700<I2c<'d, Blocking>>;

#[embassy_executor::task]
pub async fn sensor_task(
    _sda_v77: peripherals::GPIO6<'static>,
    _scl_v77: peripherals::GPIO5<'static>,
    _i2c_per: peripherals::I2C0<'static>,
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

    set_led_brightness(0);

    init_dev_info(false).await;

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

        let readings_per_call = 3;
        for i in 0..readings_per_call {
            if i > 0 {
                // Longer delay to ensure fresh VEML7700 readings.
                Timer::after_millis(100).await;
            }

            let mut v77 = get_v77();
            let lux_reading = v77.read_lux().unwrap_or(0.0);
            debug!("Raw lux: {}", lux_reading);
            lux_buf.push(lux_reading);
        }

        let buffer_count = lux_buf.len();
        let final_median_lux = lux_buf.median().unwrap_or(0.0);

        // Simple transmission density from the ambient-light sensor.
        let td_value = (final_median_lux / v77_baseline_bright) * 10.0;
        let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;
        info!(
            "td: {}, median lux: {}, baseline_bright: {}",
            adjusted_td_value, final_median_lux, v77_baseline_bright
        );

        mm_data_pub.send(Some(MeasurementData {
            td: adjusted_td_value,
            buf_count: Some(buffer_count as u32),
            hex_color: None,
        }));
    }
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
