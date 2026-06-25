use embassy_time::Timer;
use esp_hal::system::software_reset;
use heapless::String;

use crate::{
    DATA_UPDATE_CHANNEL, DEVICE_INFO_WATCH, DeviceInfo, NvsMutex, RGB_MULTIPLIERS_WATCH,
    SETTINGS_DATA_WATCH,
    helpers::{
        RGBMultipliers,
        storage::{NvsStored, Settings, WifiCreds},
    },
};

pub async fn init_signals_and_get_wifi_creds(nvs_mutex: &'static NvsMutex) -> Option<WifiCreds> {
    let settings = Settings::read(nvs_mutex)
        .await
        .expect("Couldn't read Settings")
        .unwrap_or_default();
    let rgb_m = RGBMultipliers::read(nvs_mutex)
        .await
        .expect("Couldn't read RGBMultipliers")
        .unwrap_or_default();
    let wifi = WifiCreds::read(nvs_mutex)
        .await
        .expect("Couldn't read RGBMultipliers");
    RGB_MULTIPLIERS_WATCH.sender().send(rgb_m);
    SETTINGS_DATA_WATCH.sender().send(settings);
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
