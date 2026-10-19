mod implementation;
pub mod nvs;
use defmt::Format;
pub use implementation::{NvsOpError, NvsStored};
use serde::{Deserialize, Serialize};

use crate::helpers::RGBMultipliers;
pub use td_calc::AlgoAdjustment;

#[derive(Debug, Clone, Format, Serialize, Deserialize)]
pub struct WifiCreds {
    pub ssid: heapless::String<32>,
    pub password: heapless::String<64>,
}

#[derive(Debug, Clone, Format, Serialize, Deserialize, Copy)]
pub struct Settings {
    /// in %
    pub led_brightness: u8,
    pub algo: AlgoAdjustment,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            led_brightness: 100,
            algo: AlgoAdjustment::default(),
        }
    }
}

impl NvsStored for WifiCreds {
    const KEY: &'static [u8] = b"WF_CR";
}

impl NvsStored for Settings {
    const KEY: &'static [u8] = b"SET";
}

impl NvsStored for RGBMultipliers {
    const KEY: &'static [u8] = b"RGB";
}
