#![no_std]
#![feature(impl_trait_in_assoc_type)]
#![recursion_limit = "512"]

use core::fmt::Write;
use defmt::Format;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, watch::Watch};
use heapless::String;
use serde::Serialize;

pub mod helpers;
pub mod tasks;

#[macro_export]
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

pub static CLIENT_CONNECTED: Watch<CriticalSectionRawMutex, bool, 1> = Watch::new();

pub static DEVICE_STATE: Watch<CriticalSectionRawMutex, DeviceState, 1> = Watch::new();

#[derive(Debug, Clone, Copy, Format, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceState {
    Warmup,
    FilamentInserted,
    Idle,
}

#[derive(Debug, Clone, Format, PartialEq, PartialOrd, Default, Serialize)]
pub struct MeasurementData {
    #[serde(serialize_with = "serialize_td")]
    td: f32,
    hex_color: Option<String<6>>,
    buf_count: Option<u32>,
}

fn serialize_td<S>(value: &f32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut s: String<4> = String::new();
    write!(&mut s, "{:.1}", value).unwrap();
    serializer.serialize_str(&s)
}

pub static MEASUREMENT_DATA: Watch<CriticalSectionRawMutex, Option<MeasurementData>, 2> =
    Watch::new();
