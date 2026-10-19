use defmt::{info, warn};
use embassy_time::Timer;

use crate::{
    helpers::v77,
    tasks::leds::{MeasureLedCommand, set_measure_led},
};

pub use td_calc::optics::{
    ChannelBaselines, DETECT_TRACK_ALPHA, DETECT_TRACK_FACTOR, DualReading, MeasurementResult,
    SpectralReadings, TD_EXPONENT, TD_SCALE, apply_rgb_multipliers, base_color_from_spectral,
    color_from_spectral, evaluate_measurement, format_hex, scale_rgb_color, td_from_spectral,
    td_from_transmittance, transmittance, update_detection,
};

const CHANNEL_SAMPLES: usize = 3;

/// LED levels tried during auto-exposure, from brightest to dimmest
const LEVEL_STEPS: [u8; 10] = [255, 224, 192, 160, 128, 96, 64, 48, 32, 24];
/// Air baseline count range the auto-exposure aims for: below the
/// saturation limit but high enough for good SNR on dark filaments.
const TARGET_MIN: f32 = 15000.0;
const TARGET_MAX: f32 = 55000.0;

/// Set the LED to `cmd` and return the median white-channel count.
async fn read_channel_median(veml: &mut v77::Veml7700Dev<'_>, cmd: MeasureLedCommand) -> f32 {
    set_measure_led(cmd);
    Timer::after_millis(v77::SETTLE_MS).await;
    v77::sample_median(veml, CHANNEL_SAMPLES).await
}

/// Auto-exposure: find the brightest LED level whose air reading stays
/// inside the linear target window. Returns (level, reading).
async fn auto_expose(veml: &mut v77::Veml7700Dev<'_>, kind: LedKind) -> (u8, f32) {
    let mut last = (LEVEL_STEPS[0], 0.0);
    for &level in &LEVEL_STEPS {
        let cmd = match kind {
            LedKind::Red => MeasureLedCommand::Rgb(level, 0, 0),
            LedKind::Green => MeasureLedCommand::Rgb(0, level, 0),
            LedKind::Blue => MeasureLedCommand::Rgb(0, 0, level),
            LedKind::White => MeasureLedCommand::white(level),
        };
        let reading = read_channel_median(veml, cmd).await;
        last = (level, reading);
        if reading <= TARGET_MAX {
            if reading < TARGET_MIN {
                warn!(
                    "Auto-exposure for {:?}: even level {} is below target ({})",
                    kind, level, reading
                );
            }
            return last;
        }
    }
    warn!("Auto-exposure for {:?}: all levels above target", kind);
    last
}

#[derive(Debug, Clone, Copy, defmt::Format)]
enum LedKind {
    Red,
    Green,
    Blue,
    White,
}

/// Take per-channel illumination baselines with no filament inserted.
pub async fn take_channel_baselines(veml: &mut v77::Veml7700Dev<'_>) -> ChannelBaselines {
    set_measure_led(MeasureLedCommand::Off);
    Timer::after_millis(v77::SETTLE_MS).await;
    let dark = v77::sample_median(veml, CHANNEL_SAMPLES).await;
    info!("Baseline dark: {}", dark);

    let (red_level, red_raw) = auto_expose(veml, LedKind::Red).await;
    let (green_level, green_raw) = auto_expose(veml, LedKind::Green).await;
    let (blue_level, blue_raw) = auto_expose(veml, LedKind::Blue).await;
    let (detect_level, detect_raw) = auto_expose(veml, LedKind::White).await;

    let baselines = ChannelBaselines {
        dark,
        red: (red_raw - dark).max(1.0),
        green: (green_raw - dark).max(1.0),
        blue: (blue_raw - dark).max(1.0),
        red_level,
        green_level,
        blue_level,
        detect_level,
        detect_baseline: (detect_raw - dark).max(1.0),
    };

    if !baselines.is_usable() {
        warn!("Unusable baselines: {:?}", baselines);
    }

    info!(
        "Channel baselines: R={} (level {}), G={} (level {}), B={} (level {}), detect={} (level {})",
        baselines.red,
        red_level,
        baselines.green,
        green_level,
        baselines.blue,
        blue_level,
        baselines.detect_baseline,
        detect_level
    );
    baselines
}

/// Detect whether a filament blocks the light path.
/// Returns (inserted, net reading).
pub async fn is_filament_inserted(
    veml: &mut v77::Veml7700Dev<'_>,
    baselines: &mut ChannelBaselines,
    threshold: f32,
) -> (bool, f32) {
    set_measure_led(MeasureLedCommand::Off);
    Timer::after_millis(v77::SETTLE_MS).await;
    let ambient = v77::sample_median(veml, 3).await;

    let reading =
        read_channel_median(veml, MeasureLedCommand::white(baselines.detect_level)).await;
    let net = reading - ambient;

    update_detection(net, baselines, threshold)
}
