#![no_std]
#![feature(impl_trait_in_assoc_type)]
#![recursion_limit = "512"]

use core::fmt::Write;
use defmt::Format;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, pubsub::PubSubChannel, watch::Watch,
};
use heapless::String;
use serde::{Deserialize, Serialize};

use crate::helpers::{
    RGBMultipliers,
    calibration::CalibrationCommand,
    storage::{Settings, WifiCreds, nvs::Nvs},
};

extern crate alloc;

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

pub static CLIENT_CONNECTED: Watch<CriticalSectionRawMutex, u8, 2> = Watch::new();

pub static MEASUREMENT_STATE: Watch<CriticalSectionRawMutex, MeasurementState, 1> = Watch::new();

#[derive(Debug, Clone, Copy, Format, PartialEq, Eq, PartialOrd, Ord)]
pub enum MeasurementState {
    Warmup,
    FilamentInserted,
    Idle,
}

#[derive(Debug, Clone, Copy, Format, PartialEq, Eq, PartialOrd, Ord)]
pub enum WifiState {
    HotSpotRunning,
    Connecting,
    Connected,
}

pub static WIFI_STATE: Watch<CriticalSectionRawMutex, WifiState, 1> = Watch::new();

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
    let mut s: String<8> = String::new();
    write!(&mut s, "{value:.1}").unwrap();
    serializer.serialize_str(&s)
}

pub static MEASUREMENT_DATA_WATCH: Watch<CriticalSectionRawMutex, Option<MeasurementData>, 3> =
    Watch::new();

pub static SETTINGS_DATA_WATCH: Watch<CriticalSectionRawMutex, Settings, 1> = Watch::new();

pub static RGB_MULTIPLIERS_WATCH: Watch<CriticalSectionRawMutex, RGBMultipliers, 1> = Watch::new();

pub type NvsMutex = Mutex<CriticalSectionRawMutex, Nvs>;
pub const NVS_OFFSET: usize = 0x9000;
pub const NVS_SIZE: usize = 0x6000;

#[derive(Debug, Format, Clone)]
pub enum DataUpdate {
    Settings(Settings),
    RgbMulti(RGBMultipliers),
    Wifi(WifiCreds),
}

pub static DATA_UPDATE_CHANNEL: PubSubChannel<CriticalSectionRawMutex, DataUpdate, 2, 1, 1> =
    PubSubChannel::new();

#[derive(Debug, Format, Clone, Deserialize, Serialize)]
pub struct DeviceInfo {
    has_color: bool,
    version: String<20>,
}

pub static DEVICE_INFO_WATCH: Watch<CriticalSectionRawMutex, DeviceInfo, 1> = Watch::new();

pub static CALIBRATE_REF_CHANNEL: PubSubChannel<
    CriticalSectionRawMutex,
    CalibrationCommand,
    2,
    1,
    1,
> = PubSubChannel::new();

pub static CALIBRATE_RESULT_CHANNEL: PubSubChannel<
    CriticalSectionRawMutex,
    Option<RGBMultipliers>,
    2,
    1,
    1,
> = PubSubChannel::new();
