use core::fmt::Write;
use defmt::{Format, debug, info, unwrap, warn};
use embassy_sync::pubsub::PubSubBehavior;
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
    CALIBRATE_REF_CHANNEL, CLIENT_CONNECTED, COLOR_REFERENCES_WATCH, DATA_UPDATE_CHANNEL,
    DataUpdate, MEASUREMENT_DATA_WATCH, MEASUREMENT_STATE, MeasurementData, MeasurementState,
    SETTINGS_DATA_WATCH,
    helpers::{
        calibration::{
            ColorReference, MAX_COLOR_REFERENCES, correct_color, fit_color_matrix, white_balanced,
        },
        median_buffer::RunningMedianBuffer,
        v33::take_rgb_white_balance_calibration,
        v77::{
            als_clipping_lux, configure_als, is_filament_inserted, read_als_lux,
            take_baseline_reading,
        },
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
    configure_als(&mut v77).unwrap();
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
    let mut rgb_bufs: Option<(
        RunningMedianBuffer<100>,
        RunningMedianBuffer<100>,
        RunningMedianBuffer<100>,
    )> = None;

    let mut v33 = get_v33();
    let has_color = v33.enable().is_ok();
    init_dev_info(has_color).await;

    if has_color {
        Timer::after_millis(5).await;
        rgb_white_balance = Some(take_rgb_white_balance_calibration(&mut v33).await);
        rgb_bufs = Some((
            RunningMedianBuffer::new(),
            RunningMedianBuffer::new(),
            RunningMedianBuffer::new(),
        ));
    }

    let raw_baseline = take_raw_baseline(has_color).await;
    log_raw_baseline(&raw_baseline);

    let mut lux_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();
    let mut white_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();
    let mut clear_buf: RunningMedianBuffer<100> = RunningMedianBuffer::new();

    let mut client_connected_sub = unwrap!(CLIENT_CONNECTED.receiver());
    let mut color_references_sub = COLOR_REFERENCES_WATCH.anon_receiver();
    let mut settings_sub = SETTINGS_DATA_WATCH.anon_receiver();
    let mut calib_sub = unwrap!(CALIBRATE_REF_CHANNEL.subscriber());

    let mm_data_pub = MEASUREMENT_DATA_WATCH.sender();
    dev_state_sender.send(MeasurementState::Idle);

    // Whether the filament was already detected in an earlier pass of the loop
    let mut filament_present = false;

    loop {
        if !client_connected_sub.get().await {
            set_led_brightness(0);
            // the filament might get swapped while nothing is measured
            filament_present = false;
            client_connected_sub.changed().await;
        }
        let saved_algorithm = settings_sub.try_get().expect("No Settings available").algo;

        let (is_filament_inserted, _) = {
            let mut v77 = get_v77();
            is_filament_inserted(&mut v77, v77_baseline_dark, saved_algorithm.threshold).await
        };

        if !is_filament_inserted {
            filament_present = false;
            lux_buf.clear();
            white_buf.clear();
            clear_buf.clear();
            // A calibration needs a filament. Dropping the command here makes the request time
            // out, instead of calibrating on whatever filament gets inserted later.
            while calib_sub.try_next_message_pure().is_some() {}
            if let Some((buf0, buf1, buf2)) = &mut rgb_bufs {
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

        if !filament_present {
            // The detection triggers while the filament is still being pushed in. Measure only
            // from a second later on, so no readings of a half inserted filament end up in the
            // median.
            filament_present = true;
            Timer::after_millis(1000).await;
            continue;
        }

        set_led_brightness(100);
        Timer::after_millis(300).await;

        let readings_per_call = 3;

        for i in 0..readings_per_call {
            // Longer delay to ensure fresh VEML7700 readings
            if i > 0 {
                Timer::after_millis(100).await; // Increased from 15ms to 60ms
            }

            {
                let mut v77 = get_v77();
                let lux_reading = read_als_lux(&mut v77).unwrap_or(0.0);
                debug!("Raw lux: {}", lux_reading);
                lux_buf.push(lux_reading);
                white_buf.push(v77.read_white().unwrap_or(0) as f32);
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
                    clear_buf.push(c as f32);
                }
            }
        }
        let buffer_count = lux_buf.len();

        let final_median_lux = lux_buf.median().unwrap();
        let white_transmission = ratio(white_buf.median().unwrap_or(0.0), raw_baseline.white);
        let td_value = td_from_white_transmission(white_transmission);

        if !has_color {
            let adjusted_td_value = saved_algorithm.m * td_value + saved_algorithm.b;
            mm_data_pub.send(Some(MeasurementData {
                td: adjusted_td_value,
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
        let measured_color = white_balanced((r_median_raw, g_median_raw, b_median_raw), rgb_wb);

        let mut color_references = color_references_sub
            .try_get()
            .expect("No color references available");
        if let Some(cmd) = calib_sub.try_next_message_pure() {
            let reference = ColorReference {
                measured: measured_color,
                target: [cmd.target_r, cmd.target_g, cmd.target_b],
            };
            if color_references.references.push(reference).is_ok() {
                info!("Added color reference {}", reference);
                COLOR_REFERENCES_WATCH
                    .sender()
                    .send(color_references.clone());
                DATA_UPDATE_CHANNEL
                    .publish_immediate(DataUpdate::ColorReferences(color_references.clone()));
            } else {
                warn!(
                    "Already {} color references, not adding another",
                    MAX_COLOR_REFERENCES
                );
            }
        }

        log_raw_channels(
            &RawChannels {
                lux: final_median_lux,
                white: white_buf.median().unwrap_or(0.0),
                clear: clear_buf.median().unwrap_or(0.0),
                r: r_median_raw as f32,
                g: g_median_raw as f32,
                b: b_median_raw as f32,
            },
            &raw_baseline,
        );

        info!(
            "td: {}, white transmission: {}, als transmission: {}",
            td_value,
            white_transmission,
            ratio(final_median_lux, v77_baseline_bright)
        );

        let color_matrix = fit_color_matrix(&color_references.references);
        let [r_final, g_final, b_final] = correct_color(measured_color, &color_matrix);
        info!(
            "Color: white balanced {} -> ({},{},{}) with {} references",
            measured_color,
            r_final,
            g_final,
            b_final,
            color_references.references.len()
        );

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

/// The sensors report 16 bit counts and clip at 65535.
const COUNTS_CLIPPING: f32 = 65000.0;

/// Every channel that could be used to estimate the transmission of a filament. They are only
/// logged (`RAW` lines) to find a better formula and don't influence the td value.
#[derive(Debug, Clone, Copy, Format)]
struct RawChannels {
    /// VEML7700 ALS channel in lux, this is what the td value is based on
    lux: f32,
    /// VEML7700 WHITE channel in raw counts
    white: f32,
    /// VEML3328 clear channel in raw counts
    clear: f32,
    /// VEML3328 red, green and blue channels in raw counts
    r: f32,
    g: f32,
    b: f32,
}

/// Median of every channel with nothing in the chamber, as it would read with the lamp at 100%
/// like the td baseline.
///
/// The VEML3328 clips at 100% (clear channel), so it is measured at 50% and doubled. A sweep over
/// the lamp levels showed the lamp output is proportional to its duty within 0.1%.
async fn take_raw_baseline(has_color: bool) -> RawChannels {
    const SAMPLES: usize = 10;
    let mut lux: RunningMedianBuffer<SAMPLES> = RunningMedianBuffer::new();
    let mut white: RunningMedianBuffer<SAMPLES> = RunningMedianBuffer::new();
    let mut clear: RunningMedianBuffer<SAMPLES> = RunningMedianBuffer::new();
    let mut red: RunningMedianBuffer<SAMPLES> = RunningMedianBuffer::new();
    let mut green: RunningMedianBuffer<SAMPLES> = RunningMedianBuffer::new();
    let mut blue: RunningMedianBuffer<SAMPLES> = RunningMedianBuffer::new();

    set_led_brightness(100);
    Timer::after_millis(300).await;

    for _ in 0..SAMPLES {
        let mut v77 = get_v77();
        lux.push(read_als_lux(&mut v77).unwrap_or(0.0));
        white.push(v77.read_white().unwrap_or(0) as f32);
        Timer::after_millis(100).await;
    }

    if has_color {
        set_led_brightness(50);
        Timer::after_millis(300).await;

        for _ in 0..SAMPLES {
            let mut v33 = get_v33();
            if let (Ok(r), Ok(g), Ok(b), Ok(c)) = (
                v33.read_red(),
                v33.read_green(),
                v33.read_blue(),
                v33.read_clear(),
            ) {
                red.push(r as f32);
                green.push(g as f32);
                blue.push(b as f32);
                clear.push(c as f32);
            }
            Timer::after_millis(100).await;
        }
    }

    RawChannels {
        lux: lux.median().unwrap_or(0.0),
        white: white.median().unwrap_or(0.0),
        clear: clear.median().unwrap_or(0.0) * 2.0,
        r: red.median().unwrap_or(0.0) * 2.0,
        g: green.median().unwrap_or(0.0) * 2.0,
        b: blue.median().unwrap_or(0.0) * 2.0,
    }
}

fn warn_if_clipping(what: &str, raw: &RawChannels) {
    for (name, value, limit) in [
        ("lux", raw.lux, als_clipping_lux()),
        ("white", raw.white, COUNTS_CLIPPING),
        ("clear", raw.clear, COUNTS_CLIPPING),
        ("r", raw.r, COUNTS_CLIPPING),
        ("g", raw.g, COUNTS_CLIPPING),
        ("b", raw.b, COUNTS_CLIPPING),
    ] {
        if value >= limit {
            warn!(
                "{}: channel {} is clipping ({} >= {})",
                what, name, value, limit
            );
        }
    }
}

fn log_raw_baseline(baseline: &RawChannels) {
    info!(
        "RAW baseline (empty, as at lamp 100%): lux={} white={} clear={} r={} g={} b={}",
        baseline.lux, baseline.white, baseline.clear, baseline.r, baseline.g, baseline.b
    );
    // the VEML3328 values are extrapolated from 50% and can exceed the 16 bit range on purpose
    warn_if_clipping(
        "RAW baseline",
        &RawChannels {
            clear: 0.0,
            r: 0.0,
            g: 0.0,
            b: 0.0,
            ..*baseline
        },
    );
}

/// Provisional mapping from the transmission of the VEML7700 WHITE channel to the HueForge td.
///
/// The WHITE channel is used because it is broadband. The ALS channel follows the human eye and
/// barely sees what magenta lets through (blue and red), so it read magenta 4 times darker than
/// cyan although both have about the same td.
///
/// Fitted to four Polymaker Panchroma Translucent filaments with the td printed on the spool
/// (magenta 7.8, cyan 8.0, yellow 14, natural 21), rms error 15%. The power law goes through 0
/// for an opaque filament, but below a td of about 7 it is an extrapolation.
fn td_from_white_transmission(transmission: f32) -> f32 {
    // libm instead of micromath, whose powf is off by up to 7% here
    32.67 * libm::powf(transmission.max(0.0), 0.669)
}

fn ratio(value: f32, baseline: f32) -> f32 {
    if baseline > 0.0 {
        value / baseline
    } else {
        0.0
    }
}

fn log_raw_channels(raw: &RawChannels, baseline: &RawChannels) {
    info!(
        "RAW lux={} white={} clear={} r={} g={} b={} | ratio to baseline lux={} white={} clear={} r={} g={} b={}",
        raw.lux,
        raw.white,
        raw.clear,
        raw.r,
        raw.g,
        raw.b,
        ratio(raw.lux, baseline.lux),
        ratio(raw.white, baseline.white),
        ratio(raw.clear, baseline.clear),
        ratio(raw.r, baseline.r),
        ratio(raw.g, baseline.g),
        ratio(raw.b, baseline.b),
    );
    warn_if_clipping("RAW", raw);
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
