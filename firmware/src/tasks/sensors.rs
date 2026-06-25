use core::fmt::Write;
use defmt::{Debug2Format, debug, info, unwrap};
use embassy_time::Timer;
use esp_hal::{
    Blocking,
    i2c::master::{Config, I2c},
    peripherals,
    time::Rate,
};
use heapless::String;
use veml7700::Veml7700;

use crate::{
    CALIBRATE_REF_CHANNEL, CALIBRATE_RESULT_CHANNEL, CLIENT_CONNECTED, MEASUREMENT_DATA_WATCH,
    MEASUREMENT_STATE, MeasurementData, MeasurementState, RGB_MULTIPLIERS_WATCH,
    SETTINGS_DATA_WATCH,
    helpers::{
        calibration::auto_calibrate_gray_reference,
        median_buffer::{RunningMedianBuffer, RunningMedianBufferU16},
        v33::{
            apply_rgb_multipliers, apply_spectral_response_correction,
            spectral_correction_from_rgb, take_rgb_white_balance_calibration,
        },
        v77::{is_filament_inserted, take_baseline_reading},
        veml3328::Veml3328,
    },
    tasks::{leds::set_led_brightness, states::init_dev_info},
};

pub type VEML7700<'d> = Veml7700<I2c<'d, Blocking>>;
pub type VEML3328<'d> = Veml3328<I2c<'d, Blocking>>;

#[embassy_executor::task]
pub async fn sensor_task(
    _sda_v77: peripherals::GPIO6<'static>,
    _scl_v77: peripherals::GPIO5<'static>,
    _sda_v33: peripherals::GPIO8<'static>,
    _scl_v33: peripherals::GPIO10<'static>,
    _i2c_per: peripherals::I2C0<'static>,
) {
    let dev_state_sender = MEASUREMENT_STATE.sender();
    dev_state_sender.send(MeasurementState::Warmup);

    let mut v77 = get_v77();
    v77.enable().unwrap();
    Timer::after_millis(200).await;
    set_led_brightness(100);
    info!("LED at 100%");
    Timer::after_millis(150).await;
    let v77_baseline_bright = take_baseline_reading(&mut v77).await;
    set_led_brightness(25);
    info!("LED at 25%");
    Timer::after_millis(150).await;
    let v77_baseline_dark = take_baseline_reading(&mut v77).await;

    drop(v77);
    let mut rgb_white_balance: Option<(u16, u16, u16)> = None;
    let rgb_bufs: &'static mut Option<
        (
            RunningMedianBufferU16<100>,
            RunningMedianBufferU16<100>,
            RunningMedianBufferU16<100>,
        ),
    > = crate::mk_static!(
        Option<(
            RunningMedianBufferU16<100>,
            RunningMedianBufferU16<100>,
            RunningMedianBufferU16<100>,
        )>,
        Some((
            RunningMedianBufferU16::new(),
            RunningMedianBufferU16::new(),
            RunningMedianBufferU16::new(),
        ))
    );

    let mut v33 = get_v33();
    let has_color = v33.enable().is_ok();
    init_dev_info(has_color).await;

    if has_color {
        Timer::after_millis(5).await;
        rgb_white_balance = Some(take_rgb_white_balance_calibration(&mut v33).await);
    }

    let lux_buf: &'static mut RunningMedianBuffer<100> =
        crate::mk_static!(RunningMedianBuffer<100>, RunningMedianBuffer::new());

    let mut client_connected_sub = unwrap!(CLIENT_CONNECTED.receiver());
    let mut rgb_multi_sub = RGB_MULTIPLIERS_WATCH.anon_receiver();
    let mut settings_sub = SETTINGS_DATA_WATCH.anon_receiver();
    let mut calib_sub = unwrap!(CALIBRATE_REF_CHANNEL.subscriber());

    let mm_data_pub = MEASUREMENT_DATA_WATCH.sender();
    dev_state_sender.send(MeasurementState::Idle);

    loop {
        if client_connected_sub.get().await == 0 {
            set_led_brightness(0);
            client_connected_sub.changed().await;
        }
        let saved_algorithm = settings_sub.try_get().expect("No Settings available").algo;

        let mut v77 = get_v77();
        let (filament_inserted, _) =
            is_filament_inserted(&mut v77, v77_baseline_dark, saved_algorithm.threshold).await;

        if !filament_inserted {
            drop(v77);
            lux_buf.clear();
            if let Some((buf0, buf1, buf2)) = rgb_bufs.as_mut() {
                buf0.clear();
                buf1.clear();
                buf2.clear();
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

        // Filament detected
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
                Timer::after_millis(100).await;
            }

            let lux_reading = v77.read_lux().unwrap_or(0.0);
            debug!("Raw lux: {}", lux_reading);
            lux_buf.push(lux_reading);
        }
        drop(v77);

        if has_color {
            let mut v33 = get_v33();
            for i in 0..readings_per_call {
                if i > 0 {
                    Timer::after_millis(100).await;
                }
                if let (Ok(r), Ok(g), Ok(b), Ok(c)) = (
                    v33.read_red(),
                    v33.read_green(),
                    v33.read_blue(),
                    v33.read_clear(),
                ) {
                    debug!("RGB readings {}: R={}, G={}, B={}, C={}", i + 1, r, g, b, c);

                    let buffers = rgb_bufs.as_mut().unwrap();
                    buffers.0.push(r);
                    buffers.1.push(g);
                    buffers.2.push(b);
                }
            }
        }
        let buffer_count = lux_buf.len();

        let final_median_lux = lux_buf.median().unwrap_or(0.0);

        if !has_color {
            let td_value = (final_median_lux / v77_baseline_bright) * 10.0;
            let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;
            mm_data_pub.send(Some(MeasurementData {
                td: adjusted_td_value,
                buf_count: Some(buffer_count as u32),
                hex_color: None,
            }));
            continue;
        }

        let rgb_multipliers = rgb_multi_sub
            .try_get()
            .expect("No RGB Multipliers available");
        let rgb_wb = rgb_white_balance.expect("Color sensor present but no white balance");

        if let Some(cmd) = calib_sub.try_next_message_pure() {
            info!("Calibration requested");

            let result = auto_calibrate_gray_reference(
                cmd,
                final_median_lux,
                rgb_bufs.as_mut().unwrap(),
                rgb_wb,
                rgb_multipliers,
            )
            .await;

            match result {
                Ok(cal) => {
                    RGB_MULTIPLIERS_WATCH.sender().send(cal);
                    CALIBRATE_RESULT_CHANNEL
                        .publisher()
                        .expect("calibration result publisher")
                        .publish_immediate(Some(cal));
                }
                Err(e) => {
                    info!("Calibration failed: {:?}", Debug2Format(&e));
                    CALIBRATE_RESULT_CHANNEL
                        .publisher()
                        .expect("calibration result publisher")
                        .publish_immediate(None);
                }
            }
        }

        // Do color work from here only
        let (r_median_raw, g_median_raw, b_median_raw) = {
            let buffers = rgb_bufs.as_mut().unwrap();
            (
                buffers.0.median().unwrap_or(rgb_wb.0),
                buffers.1.median().unwrap_or(rgb_wb.1),
                buffers.2.median().unwrap_or(rgb_wb.2),
            )
        };

        let spectral_correction = spectral_correction_from_rgb(r_median_raw, g_median_raw);

        let td_value = (final_median_lux / v77_baseline_bright) * spectral_correction * 10.0;
        info!(
            "td: {}, median lux: {}, baseline_bright: {}, spectral_corr: {}",
            td_value, final_median_lux, v77_baseline_bright, spectral_correction
        );

        let (r_corrected, g_corrected, b_corrected) = apply_spectral_response_correction(
            r_median_raw,
            g_median_raw,
            b_median_raw,
            rgb_wb.0,
            rgb_wb.1,
            rgb_wb.2,
        );

        // info!(
        //     "Spectral corrected RGB: ({},{},{})",
        //     r_corrected, g_corrected, b_corrected
        // );

        let (r_final, g_final, b_final) = {
            let multipliers = rgb_multipliers;
            apply_rgb_multipliers(
                r_corrected,
                g_corrected,
                b_corrected,
                final_median_lux,
                &multipliers,
            )
        };

        let mut hex_color: String<6> = String::new();
        write!(
            &mut hex_color,
            "{:02X}{:02X}{:02X}",
            r_final, g_final, b_final
        )
        .unwrap();

        let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;
        mm_data_pub.send(Some(MeasurementData {
            td: adjusted_td_value,
            buf_count: Some(buffer_count as u32),
            hex_color: Some(hex_color),
        }));
    }
}

fn get_i2c<'d>() -> I2c<'d, Blocking> {
    I2c::new(
        unsafe { peripherals::I2C0::steal() },
        Config::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
}

fn get_v33<'d>() -> VEML3328<'d> {
    let i2c = get_i2c().with_sda(v33_sda()).with_scl(v33_scl());
    VEML3328::new(i2c)
}

fn get_v77<'d>() -> Veml7700<I2c<'d, Blocking>> {
    let i2c = get_i2c().with_sda(v77_sda()).with_scl(v77_scl());
    Veml7700::new(i2c)
}

fn v77_sda() -> peripherals::GPIO6<'static> {
    unsafe { peripherals::GPIO6::steal() }
}

fn v77_scl() -> peripherals::GPIO5<'static> {
    unsafe { peripherals::GPIO5::steal() }
}

fn v33_sda() -> peripherals::GPIO8<'static> {
    unsafe { peripherals::GPIO8::steal() }
}

fn v33_scl() -> peripherals::GPIO10<'static> {
    unsafe { peripherals::GPIO10::steal() }
}
