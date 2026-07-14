use core::sync::atomic::Ordering;

use defmt::{info, warn};
use embassy_time::Timer;
use embedded_storage::Storage;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_hal::{peripherals::FLASH, system::software_reset};
use esp_storage::FlashStorage;
use heapless::String;

use crate::{
    DATA_UPDATE_CHANNEL, DEVICE_INFO_WATCH, DeviceInfo, RGB_MULTIPLIERS_WATCH, SETTINGS_DATA_WATCH,
    helpers::{
        RGBMultipliers,
        storage::{NvsStored, Settings, WifiCreds, nvs::Nvs},
    },
};

/// Marks the flash peripheral as in-use while held.
struct FlashGuard;

impl FlashGuard {
    fn acquire() -> Self {
        crate::FLASH_LOCKED.store(true, Ordering::Release);
        Self
    }
}

impl Drop for FlashGuard {
    fn drop(&mut self) {
        crate::FLASH_LOCKED.store(false, Ordering::Release);
    }
}

pub async fn init_signals_and_get_wifi_creds(
    flash: FLASH<'static>,
) -> (Option<WifiCreds>, FLASH<'static>) {
    let nvs = Nvs::new(crate::NVS_OFFSET, crate::NVS_SIZE, flash).unwrap();
    let settings = Settings::read(&nvs)
        .await
        .expect("Couldn't read Settings")
        .unwrap_or_default();
    let rgb_m = RGBMultipliers::read(&nvs)
        .await
        .expect("Couldn't read RGBMultipliers")
        .unwrap_or_default();
    let wifi = WifiCreds::read(&nvs)
        .await
        .expect("Couldn't read RGBMultipliers");
    RGB_MULTIPLIERS_WATCH.sender().send(rgb_m);
    SETTINGS_DATA_WATCH.sender().send(settings);
    (wifi, unsafe { FLASH::steal() })
}

#[embassy_executor::task]
pub async fn data_update_save_task_and_ota(_flash: FLASH<'static>) {
    let mut buf = [0u8; esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN];
    {
        let _flash_guard = FlashGuard::acquire();
        let mut flash = FlashStorage::new(unsafe { FLASH::steal() });
        let mut ota = OtaUpdater::new(&mut flash, &mut buf).unwrap();
        if let Ok(state) = ota.current_ota_state() {
            if state == esp_bootloader_esp_idf::ota::OtaImageState::New
                || state == esp_bootloader_esp_idf::ota::OtaImageState::PendingVerify
            {
                info!("Marked parition as valid");
                ota.set_current_ota_state(esp_bootloader_esp_idf::ota::OtaImageState::Valid)
                    .unwrap();
            }
        }
    }
    let mut sub = DATA_UPDATE_CHANNEL
        .subscriber()
        .expect("Couldn't subscribe to Data Update Channel");
    loop {
        if crate::OTA_IN_PROGRESS.load(Ordering::Acquire) {
            let msg = sub.next_message_pure().await;
            warn!("Dropping data update {:?} because OTA is in progress", msg);
            continue;
        }
        let msg = sub.next_message_pure().await;

        match msg {
            crate::DataUpdate::RgbMulti(d) => {
                let _flash_guard = FlashGuard::acquire();
                let nvs = get_nvs();
                d.save(&nvs).await.unwrap();
                RGB_MULTIPLIERS_WATCH.sender().send(d)
            }
            crate::DataUpdate::Settings(d) => {
                let _flash_guard = FlashGuard::acquire();
                let nvs = get_nvs();
                d.save(&nvs).await.unwrap();
                SETTINGS_DATA_WATCH.sender().send(d)
            }
            crate::DataUpdate::Wifi(d) => {
                let _flash_guard = FlashGuard::acquire();
                let nvs = get_nvs();
                d.save(&nvs).await.unwrap();
                Timer::after_millis(300).await;
                software_reset()
            }
            crate::DataUpdate::DeleteWifiCreds => {
                let _flash_guard = FlashGuard::acquire();
                let nvs = get_nvs();
                WifiCreds::delete(&nvs).await.unwrap();
            }
            crate::DataUpdate::InitUpdate => {
                let _flash_guard = FlashGuard::acquire();
                let mut flash = FlashStorage::new(unsafe { FLASH::steal() });
                let mut ota = OtaUpdater::new(&mut flash, &mut buf).unwrap();
                let (mut next_app_partition, _part_type) = ota.next_partition().unwrap();
                next_app_partition.write(2, &[0u8; 4096]).unwrap();
            }
        }
    }
}

fn get_nvs() -> Nvs {
    Nvs::new(crate::NVS_OFFSET, crate::NVS_SIZE, unsafe {
        FLASH::steal()
    })
    .unwrap()
}
pub async fn init_dev_info(has_color: bool) {
    DEVICE_INFO_WATCH.sender().send(DeviceInfo {
        has_color,
        version: String::new(),
    });
}
