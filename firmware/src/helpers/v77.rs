use defmt::{Debug2Format, error, info};
use embassy_time::Timer;
use esp_hal::{Blocking, i2c::master::I2c};
use heapless::Vec;
use td_calc::optics::DualReading;
use veml7700::{Gain, IntegrationTime, Veml7700};

pub type Veml7700Dev<'d> = Veml7700<I2c<'d, Blocking>>;

pub const SENSOR_GAIN: Gain = Gain::Two;
pub const SENSOR_IT: IntegrationTime = IntegrationTime::_100ms;

/// The sensor integrates continuously and latches its registers at the
/// end of each window. A read only `IT` after an LED change may return a
/// window that started *before* the change, mixing old and new
/// illumination. With a median of 3 samples only the first can be
/// affected, which the median filters out.
pub const SETTLE_MS: u64 = 150;

/// Delay between samples. Must exceed the integration time so that
/// consecutive samples come from *independent* integration windows -
/// otherwise the median filters nothing. Independent windows make the
/// median robust against transient drops (e.g. Wi-Fi TX current spikes
/// sagging the supply rail and dimming the LED).
pub const SAMPLE_DELAY_MS: u64 = 120;
const MAX_SAMPLES: usize = 8;

/// Counts above this indicate the sensor is saturating
pub const SATURATION_LIMIT: u16 = 60000;

pub async fn setup<'d>(
    veml: &mut Veml7700Dev<'d>,
) -> Result<(), veml7700::Error<esp_hal::i2c::master::Error>> {
    veml.enable()?;
    veml.set_integration_time(SENSOR_IT)?;
    veml.set_gain(SENSOR_GAIN)?;
    Timer::after_millis(SETTLE_MS).await;
    info!(
        "VEML7700 initialized: gain x2, integration time {} ms",
        SENSOR_IT.as_ms()
    );
    Ok(())
}

fn median_of(v: &mut Vec<f32, MAX_SAMPLES>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n.is_multiple_of(2) {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    } else {
        v[n / 2]
    }
}

/// Take `samples` simultaneous `(als, white)` readings and return their medians.
/// Recovers the sensor after read failures (disable/re-enable cycle).
pub async fn sample_dual_median(veml: &mut Veml7700Dev<'_>, samples: usize) -> DualReading {
    let mut als_readings: Vec<f32, MAX_SAMPLES> = Vec::new();
    let mut white_readings: Vec<f32, MAX_SAMPLES> = Vec::new();

    for i in 0..samples.min(MAX_SAMPLES) {
        if i > 0 {
            Timer::after_millis(SAMPLE_DELAY_MS).await;
        }
        match (veml.read_raw(), veml.read_white()) {
            (Ok(als), Ok(white)) => {
                let _ = als_readings.push(als as f32);
                let _ = white_readings.push(white as f32);
            }
            (Err(e), _) | (_, Err(e)) => {
                error!("VEML7700 read failed: {:?}", Debug2Format(&e));
                let _ = veml.disable();
                Timer::after_millis(100).await;
                let _ = veml.enable();
                Timer::after_millis(SETTLE_MS).await;
            }
        }
    }

    DualReading::new(median_of(&mut als_readings), median_of(&mut white_readings))
}

/// Take `samples` white-channel readings and return their median.
/// Recovers the sensor after read failures (disable/re-enable cycle).
pub async fn sample_median(veml: &mut Veml7700Dev<'_>, samples: usize) -> f32 {
    sample_dual_median(veml, samples).await.white
}

pub fn is_saturated(counts: u16) -> bool {
    counts >= SATURATION_LIMIT
}
