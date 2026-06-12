use core::fmt::Write;
use defmt::{debug, info, unwrap};
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
    CLIENT_CONNECTED, DEVICE_STATE, DeviceState, MEASUREMENT_DATA, MeasurementData,
    helpers::{
        RGBMultipliers,
        median_buffer::RunningMedianBuffer,
        v33::{
            apply_rgb_multipliers, apply_spectral_response_correction,
            spectral_correction_from_rgb, take_rgb_white_balance_calibration,
        },
        v77::{is_filament_inserted, take_baseline_reading},
        veml3328::Veml3328,
    },
    tasks::leds::set_led_brightness,
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
    let dev_state_sender = DEVICE_STATE.sender();
    dev_state_sender.send(DeviceState::Warmup);
    let has_color = true;

    let mut v77 = get_v77();
    Timer::after_millis(200).await;
    set_led_brightness(100);
    info!("LED at 100%");
    let v77_baseline_bright = take_baseline_reading(&mut v77).await;
    set_led_brightness(25);
    info!("LED at 25%");
    Timer::after_millis(50).await;
    let v77_baseline_dark = take_baseline_reading(&mut v77).await;

    drop(v77);
    let mut rgb_white_balance: Option<(u16, u16, u16)> = None;
    let mut rgb_bufs: Option<(
        RunningMedianBuffer<100>,
        RunningMedianBuffer<100>,
        RunningMedianBuffer<100>,
    )> = None;
    if has_color {
        let mut v33 = get_v33();
        rgb_white_balance = Some(take_rgb_white_balance_calibration(&mut v33).await);
        rgb_bufs = Some((
            RunningMedianBuffer::new(),
            RunningMedianBuffer::new(),
            RunningMedianBuffer::new(),
        ));
    }

    let rgb_multipliers = RGBMultipliers::default(); // TODO

    let mut lux_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();

    let mut client_connected_sub = unwrap!(CLIENT_CONNECTED.receiver());

    let threshold = 0.9;

    let mm_data_pub = MEASUREMENT_DATA.sender();

    loop {
        if client_connected_sub.get().await == false {
            set_led_brightness(0);
            client_connected_sub.changed().await;
        }

        let (is_filament_inserted, median_reading) = {
            let mut v77 = get_v77();
            is_filament_inserted(&mut v77, v77_baseline_dark, threshold).await
        };

        if !is_filament_inserted {
            lux_buf.clear();
            if let Some((buf0, buf1, buf2)) = &mut rgb_bufs {
                buf0.clear();
                buf1.clear();
                buf2.clear();
            }
            dev_state_sender.send_if_modified(|v| {
                let changed = *v != Some(DeviceState::Idle);
                *v = Some(DeviceState::Idle);
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
        dev_state_sender.send_if_modified(|v: &mut Option<DeviceState>| {
            let changed = *v != Some(DeviceState::FilamentInserted);
            *v = Some(DeviceState::FilamentInserted);
            changed
        });

        set_led_brightness(100);

        let readings_per_call = 3;

        for i in 0..readings_per_call {
            // Longer delay to ensure fresh VEML7700 readings
            if i > 0 {
                embassy_time::Timer::after_millis(100).await; // Increased from 15ms to 60ms
            }

            {
                let mut v77 = get_v77();
                let lux_reading = v77.read_lux().unwrap_or(0.0);
                debug!("Raw lux: {}", lux_reading);
                lux_buf.push(lux_reading);
            }
            if has_color {
                let mut v33 = get_v33();
                if let (Ok(r), Ok(g), Ok(b), Ok(c)) = (
                    v33.read_red(),
                    v33.read_green(),
                    v33.read_blue(),
                    v33.read_clear(),
                ) {
                    debug!("RGB readings {}: R={}, G={}, B={}, C={}", i + 1, r, g, b, c);

                    let buffers = rgb_bufs.as_mut().unwrap();
                    buffers.0.push(r as f32);
                    buffers.1.push(g as f32);
                    buffers.2.push(b as f32);
                }
            }
        }
        let buffer_count = lux_buf.len();

        let final_median_lux = lux_buf.median().unwrap_or(median_reading);

        if !has_color {
            let td_value = (final_median_lux / v77_baseline_bright) * 10.0;
            // let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;
            mm_data_pub.send(Some(MeasurementData {
                td: td_value,
                buf_count: Some(buffer_count as u32),
                hex_color: None,
            }));
        }

        let rgb_wb = rgb_white_balance.unwrap();

        // Do color work from here only
        let (r_median_raw, g_median_raw, b_median_raw) = {
            let buffers = rgb_bufs.as_mut().unwrap();
            (
                buffers.0.median().unwrap_or(rgb_wb.0 as f32) as u16,
                buffers.1.median().unwrap_or(rgb_wb.1 as f32) as u16,
                buffers.2.median().unwrap_or(rgb_wb.2 as f32) as u16,
            )
        };

        let spectral_correction =
            spectral_correction_from_rgb(r_median_raw as u16, g_median_raw as u16);

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

        mm_data_pub.send(Some(MeasurementData {
            td: td_value,
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
