#![no_std]
#![recursion_limit = "512"]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

//! Data collection firmware: waits for a filament to be inserted, then
//! records two full illumination sweeps, then waits again. Streams raw
//! sensor counts as CSV over the USB serial console. No Wi-Fi, no web
//! server - the radio is left off so it cannot influence the
//! measurements.
//!
//! Capture: `espflash monitor | grep --line-buffered -E "^(D|#)" > data.csv`
//! (defmt log lines are framed separately from the plain-text CSV lines).
//! `# INSERTED` / `# MEASURED` / `# REMOVED` marker lines bracket each
//! measurement, so several filaments can be recorded into one file.
//!
//! CSV format:
//! `D,<sweep>,<millis>,<led>,<r>,<g>,<b>,<sample>,<als_raw>,<white_raw>`
//! als_raw converts to lux with gain x2 / 100 ms integration (see
//! v77::SENSOR_GAIN / SENSOR_IT). Every sample is logged unfiltered -
//! all filtering and algorithm work happens offline.

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Instant, Timer};
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config, I2c},
    peripherals,
    rmt::{PulseCode, Rmt},
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_println::println;
use veml7700::Veml7700;

use firmware::helpers::{optics, v77};
use firmware::tasks::leds::{MeasureLedCommand, measure_led_task, set_measure_led};

// This creates a default app-descriptor required by the esp-idf bootloader.
esp_bootloader_esp_idf::esp_app_desc!();

/// Detection thresholds, mirroring the production detection logic
/// (optics::is_filament_inserted).
const DETECT_THRESHOLD: f32 = 0.9;
const DETECT_TRACK_FACTOR: f32 = 0.97;
const DETECT_TRACK_ALPHA: f32 = 0.2;

/// Illumination sweep: (name, r, g, b). Levels cover the LED's dynamic
/// range: full brightness (linearity/saturation data), a fine ladder
/// down to dim, and detection-like white levels. `off` states are
/// interleaved so ambient light is sampled right next to every
/// illumination.
const SEQUENCE: &[(&str, u8, u8, u8)] = &[
    ("off", 0, 0, 0),
    ("r255", 255, 0, 0),
    ("r192", 192, 0, 0),
    ("r128", 128, 0, 0),
    ("r64", 64, 0, 0),
    ("r32", 32, 0, 0),
    ("off", 0, 0, 0),
    ("g255", 0, 255, 0),
    ("g192", 0, 192, 0),
    ("g128", 0, 128, 0),
    ("g64", 0, 64, 0),
    ("g32", 0, 32, 0),
    ("off", 0, 0, 0),
    ("b255", 0, 0, 255),
    ("b192", 0, 0, 192),
    ("b128", 0, 0, 128),
    ("b64", 0, 0, 64),
    ("b32", 0, 0, 32),
    ("off", 0, 0, 0),
    ("w255", 255, 255, 255),
    ("w128", 128, 128, 128),
    ("w64", 64, 64, 64),
    ("w32", 32, 32, 32),
    ("off", 0, 0, 0),
];

const SAMPLES_PER_STATE: usize = 5;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    {
        let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
            .unwrap()
            .into_async();
        let buf = firmware::mk_static!(
            [PulseCode; esp_hal_smartled::buffer_size_async(1)],
            [PulseCode::default(); esp_hal_smartled::buffer_size_async(1)]
        );
        let measure_led =
            esp_hal_smartled::SmartLedsAdapterAsync::new(rmt.channel1, peripherals.GPIO7, buf);
        spawner.spawn(measure_led_task(measure_led).unwrap());
    }

    spawner
        .spawn(collect_task(
            peripherals.GPIO6,
            peripherals.GPIO5,
            peripherals.I2C0,
        )
        .unwrap());

    loop {
        Timer::after_secs(1).await;
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "embassy task with several match arms; the estimate is pessimistic"
)]
#[embassy_executor::task]
async fn collect_task(
    sda: peripherals::GPIO6<'static>,
    scl: peripherals::GPIO5<'static>,
    i2c_per: peripherals::I2C0<'static>,
) {
    let i2c = I2c::new(i2c_per, Config::default().with_frequency(Rate::from_khz(100)))
        .unwrap()
        .with_sda(sda)
        .with_scl(scl);
    let mut veml = Veml7700::new(i2c);

    loop {
        match v77::setup(&mut veml).await {
            Ok(()) => break,
            Err(e) => {
                info!(
                    "VEML7700 setup failed: {:?}, retrying",
                    defmt::Debug2Format(&e)
                );
                Timer::after_millis(1000).await;
            }
        }
    }

    // Give the LED task time to come up
    Timer::after_millis(500).await;

    println!("# td-free data log v3 (wait for filament, measure 2 sweeps)");
    println!(
        "# gain={:?} integration_time_ms={} settle_ms={} sample_delay_ms={} samples_per_state={}",
        v77::SENSOR_GAIN,
        v77::SENSOR_IT.as_ms(),
        v77::SETTLE_MS,
        v77::SAMPLE_DELAY_MS,
        SAMPLES_PER_STATE
    );
    println!("# columns: D,<sweep>,<millis>,<led>,<r>,<g>,<b>,<sample>,<als_raw>,<white_raw>");
    println!("# markers: # INSERTED / # MEASURED / # REMOVED bracket each measurement");

    // Reuse the production baseline procedure: auto-exposed per-channel
    // levels plus the detection level/baseline.
    let mut baselines = optics::take_channel_baselines(&mut veml).await;
    println!(
        "# boot baselines: detect_level={} detect_baseline={}",
        baselines.detect_level, baselines.detect_baseline
    );

    let mut sweep: u32 = 0;
    loop {
        // ---- wait for insertion -------------------------------------
        let mut inserted_streak = 0u32;
        loop {
            let (net, _reading) = detection_cycle(&mut veml, &mut baselines, sweep).await;
            let baseline = baselines.detect_baseline;
            if net >= baseline * DETECT_TRACK_FACTOR {
                baselines.detect_baseline = baseline * (1.0 - DETECT_TRACK_ALPHA) + net * DETECT_TRACK_ALPHA;
                inserted_streak = 0;
            } else if net <= baseline * DETECT_THRESHOLD {
                inserted_streak += 1;
            } else {
                inserted_streak = 0;
            }
            if inserted_streak >= 2 {
                break;
            }
        }
        println!("# INSERTED");

        // ---- measure: two full sweeps --------------------------------
        run_sweep(&mut veml, &mut sweep).await;
        run_sweep(&mut veml, &mut sweep).await;
        println!("# MEASURED");

        // ---- wait for removal ----------------------------------------
        let mut clear_streak = 0u32;
        loop {
            let (net, _reading) = detection_cycle(&mut veml, &mut baselines, sweep).await;
            let baseline = baselines.detect_baseline;
            if net >= baseline * DETECT_TRACK_FACTOR {
                baselines.detect_baseline = baseline * (1.0 - DETECT_TRACK_ALPHA) + net * DETECT_TRACK_ALPHA;
                clear_streak += 1;
            } else {
                clear_streak = 0;
            }
            if clear_streak >= 2 {
                break;
            }
        }
        println!("# REMOVED");
    }
}

/// One detection cycle: LED off (ambient), then detection white. Logs
/// both as CSV rows and returns the ambient-compensated (net) reading
/// plus the raw reading.
async fn detection_cycle(
    veml: &mut Veml7700<I2c<'static, esp_hal::Blocking>>,
    baselines: &mut optics::ChannelBaselines,
    sweep: u32,
) -> (f32, f32) {
    set_measure_led(MeasureLedCommand::Off);
    Timer::after_millis(v77::SETTLE_MS).await;
    let ambient = v77::sample_median(veml, 3).await;
    log_state(veml, sweep, "off", 0, 0, 0).await;

    let level = baselines.detect_level;
    set_measure_led(MeasureLedCommand::Rgb(level, level, level));
    Timer::after_millis(v77::SETTLE_MS).await;
    let reading = v77::sample_median(veml, 3).await;
    log_state(veml, sweep, "detect", level, level, level).await;

    (reading - ambient, reading)
}

/// Sample the current (already settled) illumination and log it.
async fn log_state(
    veml: &mut Veml7700<I2c<'static, esp_hal::Blocking>>,
    sweep: u32,
    name: &str,
    r: u8,
    g: u8,
    b: u8,
) {
    for i in 0..SAMPLES_PER_STATE {
        if i > 0 {
            Timer::after_millis(v77::SAMPLE_DELAY_MS).await;
        }
        let (als, white) = match (veml.read_raw(), veml.read_white()) {
            (Ok(a), Ok(w)) => (a as u32, w as u32),
            (Err(e), _) | (_, Err(e)) => {
                info!("read failed ({:?}), recovering", defmt::Debug2Format(&e));
                let _ = veml.disable();
                Timer::after_millis(100).await;
                let _ = veml.enable();
                Timer::after_millis(v77::SETTLE_MS).await;
                continue;
            }
        };
        println!(
            "D,{},{},{},{},{},{},{},{},{}",
            sweep,
            Instant::now().as_millis(),
            name,
            r,
            g,
            b,
            i,
            als,
            white
        );
    }
}

/// Run one full illumination sweep.
async fn run_sweep(veml: &mut Veml7700<I2c<'static, esp_hal::Blocking>>, sweep: &mut u32) {
    for (name, r, g, b) in SEQUENCE {
        set_measure_led(MeasureLedCommand::Rgb(*r, *g, *b));
        Timer::after_millis(v77::SETTLE_MS).await;
        log_state(veml, *sweep, name, *r, *g, *b).await;
    }
    *sweep = sweep.wrapping_add(1);
}
