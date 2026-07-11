mod implementation;
pub mod nvs;
use defmt::Format;
pub use implementation::{NvsOpError, NvsStored};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Format, Serialize, Deserialize)]
pub struct WifiCreds {
    pub ssid: heapless::String<32>,
    pub password: heapless::String<64>,
}

#[derive(Debug, Clone, Format, Serialize, Deserialize, Copy)]
pub struct AlgoAdjustment {
    pub m: f32,
    pub b: f32,
    pub threshold: f32,
}

impl Default for AlgoAdjustment {
    fn default() -> Self {
        Self {
            b: 0.0,
            m: 1.0,
            threshold: 0.9,
        }
    }
}

#[derive(Debug, Clone, Format, Serialize, Deserialize, Copy)]
pub struct Settings {
    pub algo: AlgoAdjustment,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
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
