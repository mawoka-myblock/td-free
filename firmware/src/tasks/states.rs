use embassy_time::Timer;
use esp_hal::system::software_reset;
use heapless::String;

use crate::{
    BLACK_REF_WATCH, DATA_UPDATE_CHANNEL, DEVICE_INFO_WATCH, DeviceInfo, NvsMutex,
    RGB_MULTIPLIERS_WATCH, SETTINGS_DATA_WATCH,
    helpers::{
        BlackReference, RGBMultipliers,
        storage::{NvsStored, Settings, WifiCreds},
    },
};

/// Reads an NVS-stored value, falling back to the default when the
/// stored blob can't be deserialized (e.g. after a firmware upgrade
/// changed the struct layout).
async fn read_or_default<T: NvsStored + Default>(nvs_mutex: &'static NvsMutex, name: &str) -> T {
    match T::read(nvs_mutex).await {
        Ok(Some(v)) => v,
        Ok(None) => T::default(),
        Err(e) => {
            defmt::warn!("{} unreadable ({:?}), using defaults", name, defmt::Debug2Format(&e));
            T::default()
        }
    }
}

pub async fn init_signals_and_get_wifi_creds(nvs_mutex: &'static NvsMutex) -> Option<WifiCreds> {
    let settings: Settings = read_or_default(nvs_mutex, "Settings").await;
    let rgb_m: RGBMultipliers = read_or_default(nvs_mutex, "RGBMultipliers").await;
    let black_ref: BlackReference = read_or_default(nvs_mutex, "BlackReference").await;
    let wifi = WifiCreds::read(nvs_mutex)
        .await
        .expect("Couldn't read WifiCreds");
    RGB_MULTIPLIERS_WATCH.sender().send(rgb_m);
    SETTINGS_DATA_WATCH.sender().send(settings);
    BLACK_REF_WATCH.sender().send(black_ref);
    wifi
}

#[embassy_executor::task]
pub async fn data_update_save_task(nvs_mutex: &'static NvsMutex) {
    let mut sub = DATA_UPDATE_CHANNEL
        .subscriber()
        .expect("Couldn't subscribe to Data Update Channel");
    loop {
        let msg = sub.next_message_pure().await;
        match msg {
            crate::DataUpdate::RgbMulti(d) => {
                d.save(nvs_mutex).await.unwrap();
                RGB_MULTIPLIERS_WATCH.sender().send(d)
            }
            crate::DataUpdate::BlackRef(d) => {
                d.save(nvs_mutex).await.unwrap();
                BLACK_REF_WATCH.sender().send(d)
            }
            crate::DataUpdate::Settings(d) => {
                d.save(nvs_mutex).await.unwrap();
                SETTINGS_DATA_WATCH.sender().send(d)
            }
            crate::DataUpdate::Wifi(d) => {
                d.save(nvs_mutex).await.unwrap();
                Timer::after_millis(300).await;
                software_reset()
            }
        }
    }
}

pub async fn init_dev_info(has_color: bool) {
    DEVICE_INFO_WATCH.sender().send(DeviceInfo {
        has_color,
        version: String::new(),
    });
}
