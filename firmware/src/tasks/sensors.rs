use defmt::{debug, error, info, unwrap, warn};
use embassy_sync::pubsub::PubSubBehavior;
use embassy_time::Timer;
use esp_hal::{
    Blocking,
    i2c::master::{Config, I2c},
    peripherals,
    time::Rate,
};
use veml7700::Veml7700;

use crate::{
    BLACK_CALIBRATE_CHANNEL, BLACK_REF_WATCH, CALIBRATE_REF_CHANNEL, CLIENT_CONNECTED,
    DATA_UPDATE_CHANNEL, MEASUREMENT_DATA_WATCH, MEASUREMENT_STATE, MeasurementData,
    MeasurementState, RGB_MULTIPLIERS_WATCH, SETTINGS_DATA_WATCH,
    helpers::{
        BlackReference,
        calibration::auto_calibrate_spectral,
        median_buffer::RunningMedianBuffer,
        optics::{self, DualReading, SpectralReadings},
        v77,
    },
    tasks::{
        leds::{MeasureLedCommand, set_measure_led},
        states::init_dev_info,
    },
};

pub type VEML7700<'d> = Veml7700<I2c<'d, Blocking>>;

const MEASUREMENT_CYCLES: usize = 2;
/// Re-take the illumination baselines after this many consecutive idle
/// cycles to compensate LED/sensor drift.
const IDLE_REBASELINE_CYCLES: u32 = 50;

#[embassy_executor::task]
pub async fn sensor_task(
    sda: peripherals::GPIO6<'static>,
    scl: peripherals::GPIO5<'static>,
    i2c_per: peripherals::I2C0<'static>,
) {
    let dev_state_sender = MEASUREMENT_STATE.sender();
    dev_state_sender.send(MeasurementState::Warmup);

    let i2c = I2c::new(i2c_per, Config::default().with_frequency(Rate::from_khz(100)))
        .unwrap()
        .with_sda(sda)
        .with_scl(scl);
    let mut veml = Veml7700::new(i2c);

    loop {
        match v77::setup(&mut veml).await {
            Ok(()) => break,
            Err(e) => {
                error!(
                    "VEML7700 setup failed: {:?}, retrying",
                    defmt::Debug2Format(&e)
                );
                Timer::after_millis(1000).await;
            }
        }
    }

    init_dev_info(true).await;

    // Give the LED task time to come up before driving the measurement LED
    Timer::after_millis(500).await;

    let mut baselines = optics::take_channel_baselines(&mut veml).await;

    let mut r_als_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();
    let mut r_white_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();
    let mut g_als_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();
    let mut g_white_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();
    let mut b_als_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();
    let mut b_white_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();

    let mut client_connected_sub = unwrap!(CLIENT_CONNECTED.receiver());
    let mut rgb_multi_sub = RGB_MULTIPLIERS_WATCH.anon_receiver();
    let mut settings_sub = SETTINGS_DATA_WATCH.anon_receiver();
    let mut calib_sub = unwrap!(CALIBRATE_REF_CHANNEL.subscriber());
    let mut black_calib_sub = unwrap!(BLACK_CALIBRATE_CHANNEL.subscriber());
    let mut black_ref_sub = BLACK_REF_WATCH.anon_receiver();

    let mm_data_pub = MEASUREMENT_DATA_WATCH.sender();
    let mut idle_cycles: u32 = 0;
    let mut inserted_streak: u32 = 0;
    dev_state_sender.send(MeasurementState::Idle);

    loop {
        if !client_connected_sub.get().await {
            set_measure_led(MeasureLedCommand::Off);
            client_connected_sub.changed().await;
        }
        let saved_algorithm = settings_sub.try_get().expect("No Settings available").algo;

        let (filament_blocked, detect_reading) =
            optics::is_filament_inserted(&mut veml, &mut baselines, saved_algorithm.threshold)
                .await;
        inserted_streak = if filament_blocked {
            inserted_streak + 1
        } else {
            0
        };
        info!(
            "Detection: reading={}, baseline={}, streak={}",
            detect_reading, baselines.detect_baseline, inserted_streak
        );

        if !filament_blocked || inserted_streak < 2 {
            r_als_buf.clear();
            r_white_buf.clear();
            g_als_buf.clear();
            g_white_buf.clear();
            b_als_buf.clear();
            b_white_buf.clear();
            idle_cycles += 1;

            if idle_cycles >= IDLE_REBASELINE_CYCLES {
                info!("Re-taking baselines after idle period");
                idle_cycles = 0;
                baselines = optics::take_channel_baselines(&mut veml).await;
            }

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

        idle_cycles = 0;

        // Filament detected (confirmed twice in a row)
        info!("Filament detected");
        dev_state_sender.send_if_modified(|v| {
            let changed = *v != Some(MeasurementState::FilamentInserted);
            *v = Some(MeasurementState::FilamentInserted);
            changed
        });

        for cycle in 0..MEASUREMENT_CYCLES {
            defmt::debug!("Measurement cycle {}", cycle + 1);
            // Ambient light with the LED off
            set_measure_led(MeasureLedCommand::Off);
            Timer::after_millis(v77::SETTLE_MS).await;
            let ambient = v77::sample_dual_median(&mut veml, 3).await;

            let readings = [
                (
                    MeasureLedCommand::Rgb(baselines.red_level, 0, 0),
                    &mut r_als_buf,
                    &mut r_white_buf,
                ),
                (
                    MeasureLedCommand::Rgb(0, baselines.green_level, 0),
                    &mut g_als_buf,
                    &mut g_white_buf,
                ),
                (
                    MeasureLedCommand::Rgb(0, 0, baselines.blue_level),
                    &mut b_als_buf,
                    &mut b_white_buf,
                ),
            ];
            for (cmd, als_buf, white_buf) in readings {
                set_measure_led(cmd);
                Timer::after_millis(v77::SETTLE_MS).await;
                let dual = v77::sample_dual_median(&mut veml, 3).await;
                let net = dual.net(ambient);
                debug!(
                    "Cycle {} cmd {:?} net als={}, white={}",
                    cycle + 1,
                    cmd,
                    net.als,
                    net.white
                );
                als_buf.push(net.als);
                white_buf.push(net.white);
            }
        }

        let spectral = SpectralReadings::from_baseline_levels(
            DualReading::new(
                r_als_buf.median().unwrap_or(0.0),
                r_white_buf.median().unwrap_or(0.0),
            ),
            DualReading::new(
                g_als_buf.median().unwrap_or(0.0),
                g_white_buf.median().unwrap_or(0.0),
            ),
            DualReading::new(
                b_als_buf.median().unwrap_or(0.0),
                b_white_buf.median().unwrap_or(0.0),
            ),
            &baselines,
        );

        info!(
            "Ambient-corrected medians: R=({},{}), G=({},{}), B=({},{})",
            spectral.red.als,
            spectral.red.white,
            spectral.green.als,
            spectral.green.white,
            spectral.blue.als,
            spectral.blue.white
        );

        if let Some(()) = black_calib_sub.try_next_message_pure() {
            info!(
                "Black reference captured: R={}, G={}, B={}",
                spectral.red.white, spectral.green.white, spectral.blue.white
            );
            DATA_UPDATE_CHANNEL.publish_immediate(crate::DataUpdate::BlackRef(BlackReference {
                r: spectral.red.white,
                g: spectral.green.white,
                b: spectral.blue.white,
            }));
            Timer::after_millis(50).await;
        }

        let black_ref = black_ref_sub
            .try_get()
            .expect("No black reference available");

        let mut rgb_multipliers = rgb_multi_sub
            .try_get()
            .expect("No RGB Multipliers available");

        let prelim = optics::evaluate_measurement(
            &spectral,
            &baselines,
            &black_ref,
            &saved_algorithm,
            &rgb_multipliers,
        );

        // Sanity check: a supposedly inserted filament that transmits
        // almost everything means the baselines are stale (or detection
        // misfired). Re-baseline instead of publishing garbage.
        let t = prelim.transmittance;
        if (t.0 + t.1 + t.2) / 3.0 > 0.9 {
            warn!("Measurement looks like an empty slot (T mean > 0.9), re-baselining");
            baselines = optics::take_channel_baselines(&mut veml).await;
            mm_data_pub.send_if_modified(|v| {
                let changed = *v != Some(None);
                *v = Some(None);
                changed
            });
            continue;
        }

        if let Some(cmd) = calib_sub.try_next_message_pure() {
            info!("Calibration requested");

            match auto_calibrate_spectral(
                cmd,
                &spectral,
                &baselines,
                prelim.raw_td,
                rgb_multipliers,
            ) {
                Ok(cal) => {
                    RGB_MULTIPLIERS_WATCH.sender().send(cal);
                    rgb_multipliers = cal;
                }
                Err(e) => {
                    info!("Calibration failed: {:?}", defmt::Debug2Format(&e));
                }
            }
        }

        let result = optics::evaluate_measurement(
            &spectral,
            &baselines,
            &black_ref,
            &saved_algorithm,
            &rgb_multipliers,
        );
        info!(
            "td: {} (raw {}), hex: #{}, transmittance: ({}, {}, {})",
            result.td, result.raw_td, result.hex_color, t.0, t.1, t.2
        );

        mm_data_pub.send(Some(MeasurementData {
            td: result.td,
            buf_count: Some(r_white_buf.len() as u32),
            hex_color: Some(result.hex_color),
        }));
    }
}
