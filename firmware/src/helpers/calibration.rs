use defmt::Format;
use serde::{Deserialize, Serialize};

/// Adds the inserted filament as a color reference with this color
#[derive(Debug, Format, Clone, Copy, Deserialize, Serialize)]
pub struct CalibrationCommand {
    pub target_r: u8,
    pub target_g: u8,
    pub target_b: u8,
}

#[derive(Debug, Format, Clone, Copy, Deserialize, Serialize)]
pub struct RemoveReferenceCommand {
    pub index: u8,
}

pub const MAX_COLOR_REFERENCES: usize = 8;

/// A filament with a known color: what the sensor measured and what it should show.
#[derive(Debug, Format, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct ColorReference {
    /// White balanced sensor color, see `white_balanced`
    pub measured: [f32; 3],
    /// sRGB color printed on the spool
    pub target: [u8; 3],
}

#[derive(Debug, Format, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct ColorReferences {
    pub references: heapless::Vec<ColorReference, MAX_COLOR_REFERENCES>,
}

/// One reference as the calibration page shows it
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ColorReferenceView {
    pub target: [u8; 3],
    /// Color with the white balance only, without correction
    pub measured: [u8; 3],
    /// Color the device shows now, with the correction from all references
    pub corrected: [u8; 3],
}

/// Maps the white balanced sensor color (linear light) to the displayed color (linear light)
pub type ColorMatrix = [[f32; 3]; 3];

const IDENTITY: ColorMatrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// Pulls the full fit towards the per-channel solution. With only a few references a free 3x3
/// fit overfits: tested on four Panchroma spools, a spool left out of the fit was off by ΔE 32
/// without it and ΔE 22 with it, while the fitted spools stayed at ΔE 8.
const RIDGE: f32 = 0.025;

/// Scales the raw sensor channels so that the empty chamber (`wb`, measured at power-up) reads
/// the same on all three, the lamp is white by definition.
///
/// Bright filaments are then scaled so the strongest channel is 1, which keeps only their hue:
/// a translucent filament looks bright although it lets through only a few percent of the light.
/// Very dark filaments keep their brightness.
pub fn white_balanced(raw: (u16, u16, u16), wb: (u16, u16, u16)) -> [f32; 3] {
    let raw = [raw.0 as f32, raw.1 as f32, raw.2 as f32];
    let wb = [wb.0 as f32, wb.1 as f32, wb.2 as f32];
    if wb.iter().any(|&w| w <= 0.0) {
        return [0.5; 3];
    }
    let total = wb[0] + wb[1] + wb[2];
    let balanced = [0, 1, 2].map(|i| raw[i] * total / (3.0 * wb[i]));
    let max = balanced[0].max(balanced[1]).max(balanced[2]);
    let scale = if max > 255.0 { 1.0 / max } else { 1.0 / 255.0 };
    balanced.map(|v| v * scale)
}

/// Fits the color matrix to the references. No reference: the white balance only. One or two:
/// a factor per channel (the white point). Three or more: a full 3x3 matrix, which also corrects
/// how the channels bleed into each other.
pub fn fit_color_matrix(references: &[ColorReference]) -> ColorMatrix {
    if references.is_empty() {
        return IDENTITY;
    }
    let targets: heapless::Vec<[f32; 3], MAX_COLOR_REFERENCES> = references
        .iter()
        .map(|r| r.target.map(srgb_to_linear))
        .collect();

    let mut matrix = [[0.0; 3]; 3];
    for ch in 0..3 {
        // per-channel factor, least squares
        let (sxy, sxx) = references
            .iter()
            .zip(&targets)
            .fold((0.0, 0.0), |(sxy, sxx), (r, t)| {
                (
                    sxy + r.measured[ch] * t[ch],
                    sxx + r.measured[ch] * r.measured[ch],
                )
            });
        let per_channel = if sxx > 0.0 { sxy / sxx } else { 1.0 };
        matrix[ch][ch] = per_channel;
        if references.len() < 3 {
            continue;
        }

        // full row: least squares with a ridge towards the per-channel factor
        let mut ata = [[0.0; 3]; 3];
        let mut atb = [0.0; 3];
        for (r, t) in references.iter().zip(&targets) {
            for i in 0..3 {
                atb[i] += r.measured[i] * t[ch];
                for j in 0..3 {
                    ata[i][j] += r.measured[i] * r.measured[j];
                }
            }
        }
        for (i, row) in ata.iter_mut().enumerate() {
            row[i] += RIDGE;
        }
        atb[ch] += RIDGE * per_channel;
        if let Some(row) = solve3(ata, atb) {
            matrix[ch] = row;
        }
    }
    matrix
}

pub fn correct_color(measured: [f32; 3], matrix: &ColorMatrix) -> [u8; 3] {
    matrix.map(|row| {
        linear_to_srgb(row[0] * measured[0] + row[1] * measured[1] + row[2] * measured[2])
    })
}

pub fn reference_views(
    references: &[ColorReference],
) -> heapless::Vec<ColorReferenceView, MAX_COLOR_REFERENCES> {
    let matrix = fit_color_matrix(references);
    references
        .iter()
        .map(|r| ColorReferenceView {
            target: r.target,
            measured: correct_color(r.measured, &IDENTITY),
            corrected: correct_color(r.measured, &matrix),
        })
        .collect()
}

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        libm::powf((c + 0.055) / 1.055, 2.4)
    }
}

fn linear_to_srgb(v: f32) -> u8 {
    let c = v.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * libm::powf(c, 1.0 / 2.4) - 0.055
    };
    libm::roundf(s * 255.0).clamp(0.0, 255.0) as u8
}

fn det3(m: &[[f32; 3]; 3]) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

/// Solves a x = b with Cramer's rule, None if a is singular
fn solve3(a: [[f32; 3]; 3], b: [f32; 3]) -> Option<[f32; 3]> {
    let d = det3(&a);
    if libm::fabsf(d) < 1e-9 {
        return None;
    }
    Some([0, 1, 2].map(|col| {
        let mut m = a;
        for (row, value) in m.iter_mut().zip(b) {
            row[col] = value;
        }
        det3(&m) / d
    }))
}
