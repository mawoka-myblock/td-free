use core::fmt::Write;
#[cfg(feature = "defmt")]
use defmt::Format;
use heapless::String;
use micromath::F32Ext;

use crate::{AlgoAdjustment, BlackReference, RGBMultipliers};

/// td is reported on the HueForge scale: 0.1 (opaque) to 100
/// (fully transparent).
pub const TD_SCALE: f32 = 100.0;

/// Legacy single-channel td mapping exponent.
pub const TD_EXPONENT: f32 = 0.7;

/// Dual-band spectral td mapping exponent and scale factor.
/// Fitted from VEML7700 (ALS + WHITE) measurements of reference filaments
/// (Black, Teal 0.6, Red 1.0, Lime 2.3, Yellow 4.3, Magenta 7.8).
const SPECTRAL_TD_EXPONENT: f32 = 0.46;
const SPECTRAL_TD_SCALE: f32 = 0.20;

/// Detection: readings at or above this fraction of the baseline are
/// treated as clearly unobstructed and used to slowly re-track the
/// baseline (compensates LED aging, temperature and relocation).
pub const DETECT_TRACK_FACTOR: f32 = 0.97;
/// Weight of a new unobstructed reading in the baseline EMA.
pub const DETECT_TRACK_ALPHA: f32 = 0.2;

/// Native VEML7700 ALS/WHITE ratios for pure LED wavelengths:
/// - Red LED (~625 nm): ALS/WHITE ≈ 0.396
/// - Blue LED (~470 nm): ALS/WHITE ≈ 0.964
const ALS_WHITE_RATIO_RED: f32 = 0.396;
const ALS_WHITE_RATIO_BLUE: f32 = 0.964;

/// Geometric scatter coupling fractions relative to unshifted blue scatter
/// in baseline-normalized transmittance units.
const SCATTER_COUPLING_RED: f32 = 0.135;
const SCATTER_COUPLING_GREEN: f32 = 0.115;

/// Default dark floor in baseline-normalized transmittance units (~15 counts / 55000).
const DEFAULT_DARK_FLOOR_R: f32 = 15.0 / 60912.0;
const DEFAULT_DARK_FLOOR_G: f32 = 15.0 / 54182.0;

/// Normalized transmittance scatter floors for color extraction
/// (removes white TiO2 surface/slot scatter before computing chromaticity).
const COLOR_FLOOR_R: f32 = 170.0 / 60912.0;
const COLOR_FLOOR_G: f32 = 111.0 / 54182.0;
const COLOR_FLOOR_B: f32 = 765.0 / 53269.0;

/// Total normalized white transmittance below which a filament is classified as black.
const BLACK_TOTAL_TRANSMITTANCE_THRESHOLD: f32 = 0.0018;

/// Illumination baselines taken without a filament inserted, plus the
/// per-channel LED levels the baselines were taken at (auto-exposed to
/// keep the sensor in its linear range). All baseline values are
/// ambient-corrected white-channel counts.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct ChannelBaselines {
    pub dark: f32,
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub red_level: u8,
    pub green_level: u8,
    pub blue_level: u8,
    pub detect_level: u8,
    /// Ambient-compensated (net) detection baseline
    pub detect_baseline: f32,
}

impl ChannelBaselines {
    pub fn is_usable(&self) -> bool {
        self.red > 1.0 && self.green > 1.0 && self.blue > 1.0
    }

    /// Default reference baselines matching the logged hardware calibration
    /// (`R=53507 @ 224`, `G=54182 @ 255`, `B=53269 @ 255`, `detect=54694 @ 96`).
    pub const fn default_hardware() -> Self {
        Self {
            dark: 18.0,
            red: 53507.0,
            green: 54182.0,
            blue: 53269.0,
            red_level: 224,
            green_level: 255,
            blue_level: 255,
            detect_level: 96,
            detect_baseline: 54694.0,
        }
    }

    /// Effective open-air white baseline scaled to arbitrary LED drive levels `(r_lvl, g_lvl, b_lvl)`.
    pub fn scaled_for_levels(&self, r_lvl: u8, g_lvl: u8, b_lvl: u8) -> (f32, f32, f32) {
        let r_scale = r_lvl as f32 / (self.red_level.max(1) as f32);
        let g_scale = g_lvl as f32 / (self.green_level.max(1) as f32);
        let b_scale = b_lvl as f32 / (self.blue_level.max(1) as f32);
        (
            (self.red * r_scale).max(1.0),
            (self.green * g_scale).max(1.0),
            (self.blue * b_scale).max(1.0),
        )
    }
}

impl Default for ChannelBaselines {
    fn default() -> Self {
        Self::default_hardware()
    }
}

/// Simultaneous VEML7700 photodiode reading: narrowband photopic `als`
/// (`read_raw()`) and broadband `white` (`read_white()`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct DualReading {
    pub als: f32,
    pub white: f32,
}

impl DualReading {
    pub const fn new(als: f32, white: f32) -> Self {
        Self { als, white }
    }

    pub const fn white_only(white: f32) -> Self {
        Self { als: 0.0, white }
    }

    /// Subtract ambient dark reading, clamping at 0.
    pub fn net(self, ambient: DualReading) -> Self {
        Self {
            als: (self.als - ambient.als).max(0.0),
            white: (self.white - ambient.white).max(0.0),
        }
    }
}

/// Ambient-compensated per-channel readings for a single R/G/B measurement cycle,
/// together with the LED levels used during the measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct SpectralReadings {
    pub red: DualReading,
    pub green: DualReading,
    pub blue: DualReading,
    pub red_level: u8,
    pub green_level: u8,
    pub blue_level: u8,
}

impl SpectralReadings {
    /// Construct from ambient-corrected `(als, white)` readings taken at the baseline LED levels.
    pub fn from_baseline_levels(
        red: DualReading,
        green: DualReading,
        blue: DualReading,
        baselines: &ChannelBaselines,
    ) -> Self {
        Self {
            red,
            green,
            blue,
            red_level: baselines.red_level,
            green_level: baselines.green_level,
            blue_level: baselines.blue_level,
        }
    }

    /// Construct from white-only ambient-corrected counts at baseline LED levels.
    pub fn from_white_only(
        reading: (f32, f32, f32),
        baselines: &ChannelBaselines,
    ) -> Self {
        Self::from_baseline_levels(
            DualReading::white_only(reading.0),
            DualReading::white_only(reading.1),
            DualReading::white_only(reading.2),
            baselines,
        )
    }

    pub fn has_als(&self) -> bool {
        self.red.als > 0.0 || self.green.als > 0.0 || self.blue.als > 0.0
    }
}

/// Complete output of the offline/online measurement calculation.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "defmt", derive(Format))]
pub struct MeasurementResult {
    /// Final TD value after `m * raw_td + b` adjustment.
    pub td: f32,
    /// Unadjusted TD value on the HueForge scale (0.1..100).
    pub raw_td: f32,
    /// Final 8-bit RGB color after calibration multipliers.
    pub rgb: (u8, u8, u8),
    /// Base 8-bit RGB color before user calibration multipliers.
    pub base_rgb: (u8, u8, u8),
    /// Formatted 6-character uppercase hex color (without `#`).
    pub hex_color: String<6>,
    /// Per-channel white transmittance `(T_r, T_g, T_b)`.
    pub transmittance: (f32, f32, f32),
}

/// Pure filament-detection state update.
/// Given the ambient-compensated white reading `net`, updates `baselines.detect_baseline`
/// if the path is unobstructed and returns `(inserted, net)`.
pub fn update_detection(
    net: f32,
    baselines: &mut ChannelBaselines,
    threshold: f32,
) -> (bool, f32) {
    let baseline = baselines.detect_baseline;
    if baseline <= 0.0 {
        baselines.detect_baseline = net.max(1.0);
        return (false, net);
    }

    if net >= baseline * DETECT_TRACK_FACTOR {
        baselines.detect_baseline =
            baseline * (1.0 - DETECT_TRACK_ALPHA) + net * DETECT_TRACK_ALPHA;
        return (false, net);
    }

    let current_threshold = baseline * threshold.clamp(0.0, 1.0);
    (net <= current_threshold, net)
}

/// Convert ambient-corrected white-channel counts into per-channel
/// transmittance (0.0 = fully blocked, 1.0 = fully transmitted).
/// The readings must already have the per-cycle ambient light subtracted.
/// If a black reference is set, its counts (light leaking around the
/// filament) are subtracted first.
pub fn transmittance(
    reading: (f32, f32, f32),
    baselines: &ChannelBaselines,
    black: &BlackReference,
) -> (f32, f32, f32) {
    let (r, g, b) = if black.is_enabled() {
        (
            reading.0 - black.r,
            reading.1 - black.g,
            reading.2 - black.b,
        )
    } else {
        reading
    };
    (
        (r / baselines.red.max(1.0)).clamp(0.0, 2.0),
        (g / baselines.green.max(1.0)).clamp(0.0, 2.0),
        (b / baselines.blue.max(1.0)).clamp(0.0, 2.0),
    )
}

/// Weighted mean of the per-channel transmittances, mapped onto the
/// td scale via a power law (legacy white-only formula).
pub fn td_from_transmittance(t: (f32, f32, f32), weights: (f32, f32, f32)) -> f32 {
    let sum = weights.0 + weights.1 + weights.2;
    let mean = if sum > 0.0 {
        (t.0 * weights.0 + t.1 * weights.1 + t.2 * weights.2) / sum
    } else {
        (t.0 + t.1 + t.2) / 3.0
    };
    F32Ext::powf(mean, TD_EXPONENT) * TD_SCALE
}

/// Compute TD from dual-band `(ALS, WHITE)` spectral readings.
///
/// Uses the VEML7700 ALS/WHITE ratio under blue illumination to separate
/// geometric surface/slot scatter (`b_scat`, unshifted 470 nm light) from
/// wavelength-shifted dye fluorescence/transmission (`b_fluor`), then subtracts
/// the proportional geometric scatter from the red and green channels.
pub fn td_from_spectral(
    readings: &SpectralReadings,
    baselines: &ChannelBaselines,
    black: &BlackReference,
    weights: (f32, f32, f32),
) -> f32 {
    let (base_r, base_g, base_b) = baselines.scaled_for_levels(
        readings.red_level,
        readings.green_level,
        readings.blue_level,
    );

    let rw = (readings.red.white / base_r).max(0.0);
    let gw = (readings.green.white / base_g).max(0.0);
    let bw = (readings.blue.white / base_b).max(0.0);

    // Total transmittance below the black threshold indicates a completely opaque/black filament.
    if rw + gw + bw < BLACK_TOTAL_TRANSMITTANCE_THRESHOLD {
        return 0.1;
    }

    let (dark_r, dark_g) = if black.is_enabled() {
        (
            (black.r / baselines.red.max(1.0)).max(0.0),
            (black.g / baselines.green.max(1.0)).max(0.0),
        )
    } else {
        (DEFAULT_DARK_FLOOR_R, DEFAULT_DARK_FLOOR_G)
    };

    let (b_scat, b_fluor) = if readings.has_als() {
        let ba = (readings.blue.als / base_b).max(0.0);
        let scat = ((ba - ALS_WHITE_RATIO_RED * bw)
            / (ALS_WHITE_RATIO_BLUE - ALS_WHITE_RATIO_RED))
            .max(0.0);
        let fluor = (bw - scat).max(0.0);
        (scat, fluor)
    } else {
        // Fallback when ALS is unavailable: estimate scatter from COLOR_FLOOR_B.
        let scat = bw.min(COLOR_FLOOR_B);
        let fluor = (bw - COLOR_FLOOR_B).max(0.0);
        (scat, fluor)
    };

    let wr = weights.0.max(0.0);
    let wg = weights.1.max(0.0);
    let wb = weights.2.max(0.0);

    let r_corr = (rw - dark_r - SCATTER_COUPLING_RED * b_scat).max(0.0) * wr;
    let g_corr = (gw - dark_g - SCATTER_COUPLING_GREEN * b_scat).max(0.0) * 1.2 * wg;

    let t_eff = r_corr.max(g_corr) + 3.5 * wb * b_fluor;
    if t_eff <= 0.0 {
        return 0.1;
    }

    (SPECTRAL_TD_SCALE * F32Ext::powf(t_eff, SPECTRAL_TD_EXPONENT) * TD_SCALE).clamp(0.1, TD_SCALE)
}

/// Reconstruct the base 8-bit RGB filament color from dual-band `(ALS, WHITE)`
/// spectral readings before user calibration multipliers.
///
/// Compensates:
/// 1. White TiO2 / slot geometric scatter floor on R, G, B.
/// 2. Cyan/green passband leakage of the 470 nm Blue LED on yellow-green filaments.
/// 3. Wavelength shift (ALS/WHITE ratio drop) under 525 nm Green and 470 nm Blue
///    illumination caused by yellow and magenta pigments.
/// 4. Beer-Lambert / Mie multiple-scattering dynamic range compression.
pub fn base_color_from_spectral(
    readings: &SpectralReadings,
    baselines: &ChannelBaselines,
) -> (u8, u8, u8) {
    let (base_r, base_g, base_b) = baselines.scaled_for_levels(
        readings.red_level,
        readings.green_level,
        readings.blue_level,
    );

    let rw = (readings.red.white / base_r).max(0.0);
    let gw = (readings.green.white / base_g).max(0.0);
    let bw = (readings.blue.white / base_b).max(0.0);

    if rw + gw + bw < BLACK_TOTAL_TRANSMITTANCE_THRESHOLD {
        return (0, 0, 0);
    }

    let dr = (rw - COLOR_FLOOR_R).max(0.0);
    let dg = (gw - COLOR_FLOOR_G).max(0.0);
    // Subtract yellow-green passband crosstalk under the 470 nm LED.
    let db = (bw - COLOR_FLOOR_B - 1.50 * dr.min(dg)).max(0.0);

    let (g_shift, b_shift) = if readings.has_als() {
        let ga = (readings.green.als / base_g).max(0.0);
        let ba = (readings.blue.als / base_b).max(0.0);
        (
            (1.90 * gw - ga).max(0.0),
            (0.92 * bw - ba).max(0.0),
        )
    } else {
        (0.0, 0.0)
    };

    let r_val = dr;
    let g_val = dg * 1.25 + g_shift * 26.0;
    let b_val = db * 0.30 + b_shift * 14.5;

    let max_val = r_val.max(g_val).max(b_val);
    if max_val <= 1e-6 {
        return (0, 0, 0);
    }

    let rn = F32Ext::powf(r_val / max_val, 0.85);
    let gn = F32Ext::powf(g_val / max_val, 0.85);
    let bn = F32Ext::powf(b_val / max_val, 0.85);

    let scale = (218.0 + 7.5 * F32Ext::log10((max_val / 0.00025).max(1.0))).clamp(0.0, 255.0);

    (
        F32Ext::round(rn * scale).clamp(0.0, 255.0) as u8,
        F32Ext::round(gn * scale).clamp(0.0, 255.0) as u8,
        F32Ext::round(bn * scale).clamp(0.0, 255.0) as u8,
    )
}

/// Apply user calibration multipliers to a base 8-bit `(R, G, B)` color.
pub fn scale_rgb_color(base_rgb: (u8, u8, u8), m: &RGBMultipliers) -> (u8, u8, u8) {
    let r = F32Ext::round(base_rgb.0 as f32 * m.brightness * m.red).clamp(0.0, 255.0) as u8;
    let g = F32Ext::round(base_rgb.1 as f32 * m.brightness * m.green).clamp(0.0, 255.0) as u8;
    let b = F32Ext::round(base_rgb.2 as f32 * m.brightness * m.blue).clamp(0.0, 255.0) as u8;
    (r, g, b)
}

/// Compute calibrated 8-bit RGB color from dual-band spectral readings.
pub fn color_from_spectral(
    readings: &SpectralReadings,
    baselines: &ChannelBaselines,
    multipliers: &RGBMultipliers,
) -> (u8, u8, u8) {
    let base = base_color_from_spectral(readings, baselines);
    scale_rgb_color(base, multipliers)
}

/// Legacy helper: map transmittance `(T_r, T_g, T_b)` to an 8-bit RGB color
/// using the calibration multipliers.
pub fn apply_rgb_multipliers(t: (f32, f32, f32), m: &RGBMultipliers) -> (u8, u8, u8) {
    // If transmittance is small (< 0.25, typical for real filaments), reconstruct
    // base chromaticity first so opaque filaments do not collapse to black.
    let baselines = ChannelBaselines::default_hardware();
    let readings = SpectralReadings::from_white_only(
        (
            t.0 * baselines.red,
            t.1 * baselines.green,
            t.2 * baselines.blue,
        ),
        &baselines,
    );
    if t.0 + t.1 + t.2 > 0.0 && t.0.max(t.1).max(t.2) < 0.25 {
        let base = base_color_from_spectral(&readings, &baselines);
        if base != (0, 0, 0) {
            return scale_rgb_color(base, m);
        }
    }

    let scale = 255.0 * m.brightness;
    let r = F32Ext::round(t.0 * scale * m.red).clamp(0.0, 255.0) as u8;
    let g = F32Ext::round(t.1 * scale * m.green).clamp(0.0, 255.0) as u8;
    let b = F32Ext::round(t.2 * scale * m.blue).clamp(0.0, 255.0) as u8;
    (r, g, b)
}

/// Format an `(R, G, B)` tuple as a 6-character uppercase hex string (without `#`).
pub fn format_hex(rgb: (u8, u8, u8)) -> String<6> {
    let mut s = String::new();
    let _ = write!(&mut s, "{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2);
    s
}

/// Run the full TD and color calculation pipeline on a set of spectral readings.
pub fn evaluate_measurement(
    readings: &SpectralReadings,
    baselines: &ChannelBaselines,
    black: &BlackReference,
    algo: &AlgoAdjustment,
    multipliers: &RGBMultipliers,
) -> MeasurementResult {
    let (base_r, base_g, base_b) = baselines.scaled_for_levels(
        readings.red_level,
        readings.green_level,
        readings.blue_level,
    );
    let scaled_baselines = ChannelBaselines {
        red: base_r,
        green: base_g,
        blue: base_b,
        ..*baselines
    };
    let t = transmittance(
        (readings.red.white, readings.green.white, readings.blue.white),
        &scaled_baselines,
        black,
    );

    let raw_td = td_from_spectral(readings, baselines, black, (algo.wr, algo.wg, algo.wb));
    let td = (algo.m * raw_td + algo.b).max(0.1);
    let base_rgb = base_color_from_spectral(readings, baselines);
    let rgb = scale_rgb_color(base_rgb, multipliers);
    let hex_color = format_hex(rgb);

    MeasurementResult {
        td,
        raw_td,
        rgb,
        base_rgb,
        hex_color,
        transmittance: t,
    }
}
