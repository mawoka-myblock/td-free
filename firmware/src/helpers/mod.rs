use defmt::Format;
use serde::{Deserialize, Serialize};

pub mod calibration;
pub mod median_buffer;
pub mod storage;
pub mod v33;
pub mod v77;
pub mod veml3328;

#[derive(Debug, Clone, Copy, Format, Deserialize, Serialize)]
pub struct RGBMultipliers {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub brightness: f32,
    /// No longer used, the brightness isn't scaled by the lux anymore. Kept so that stored
    /// settings and API clients keep working.
    pub td_reference: f32,
    pub reference_r: u8, // Reference red value (0-255)
    pub reference_g: u8, // Reference green value (0-255)
    pub reference_b: u8, // Reference blue value (0-255)
}

impl Default for RGBMultipliers {
    fn default() -> Self {
        Self {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            brightness: 1.0,
            td_reference: 0.0,
            reference_r: 127, // Default to 50% grey
            reference_g: 127, // Default to 50% grey
            reference_b: 127, // Default to 50% grey
        }
    }
}
