use crate::helpers::storage::NvsStored;

pub mod calibration;
pub mod median_buffer;
pub mod optics;
pub mod storage;
pub mod v77;

pub use td_calc::{BlackReference, RGBMultipliers};

impl NvsStored for BlackReference {
    const KEY: &'static [u8] = b"BLKRF";
}
