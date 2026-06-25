use alloc::rc::Rc;
use defmt::{Debug2Format, info};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex,
    mutex::Mutex,
    semaphore::{GreedySemaphore, Semaphore},
};
use embedded_storage::{ReadStorage, Storage};
use esp_hal::rom::crc::crc32_be;
use portable_atomic::AtomicU8;
use tickv::{ErrorCode, FlashController};

static mut NVS_READ_BUF: &mut [u8; 1024] = &mut [0; 1024];
static NVS_INSTANCES: AtomicU8 = AtomicU8::new(0);

pub struct Nvs {
    flash_peripheral: esp_hal::peripherals::FLASH<'static>,
    tickv: Rc<tickv::TicKV<'static, NvsFlash, 1024>>,
    semaphore: Rc<GreedySemaphore<CriticalSectionRawMutex>>,

    offset: usize,
    size: usize,
}

impl Nvs {
    pub fn new(
        flash_offset: usize,
        flash_size: usize,
        flash: esp_hal::peripherals::FLASH<'static>,
    ) -> anyhow::Result<Self, ErrorCode> {
        if NVS_INSTANCES.load(core::sync::atomic::Ordering::Relaxed) > 0 {
            defmt::error!("Cannot spawn new NVS struct, clone original one instead!");
            return Err(ErrorCode::KeyNotFound);
        }

        NVS_INSTANCES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        unsafe { Self::new_unchecked(flash_offset, flash_size, flash) }
    }

    /// # Safety
    ///
    /// This is not checking if other nvs instance already exists (there should be only one nvs
    /// instancce!)
    pub unsafe fn new_unchecked(
        flash_offset: usize,
        flash_size: usize,
        flash: esp_hal::peripherals::FLASH<'static>,
    ) -> anyhow::Result<Self, ErrorCode> {
        let nvs = tickv::TicKV::<NvsFlash, 1024>::new(
            NvsFlash::new(flash_offset, unsafe { flash.clone_unchecked() }),
            unsafe { NVS_READ_BUF },
            flash_size,
        );
        nvs.initialise(hash(tickv::MAIN_KEY))?;

        Ok(Nvs {
            flash_peripheral: flash,
            tickv: Rc::new(nvs),
            semaphore: Rc::new(GreedySemaphore::new(1)),

            offset: flash_offset,
            size: flash_size,
        })
    }

    pub async fn get_key(&self, key: &[u8]) -> anyhow::Result<[u8; 1024], ErrorCode> {
        let _permit = self.semaphore.acquire(1).await.unwrap();
        let mut buf = [0u8; 1024];
        self.tickv.get_key(hash(key), &mut buf)?;
        Ok(buf)
    }

    pub async fn append_key(&self, key: &[u8], buf: &[u8]) -> Result<(), ErrorCode> {
        let _drop = self.semaphore.acquire(1).await.unwrap();
        let res = self.tickv.append_key(hash(key), buf);
        if let Err(e) = res {
            info!("Tickv error!");
            info!("Error: {:?}", Debug2Format(&e));
            if e == ErrorCode::UnsupportedVersion {
                defmt::error!(
                    "Unsupported version while appending flash key... Wiping NVS partition!"
                );

                self.wipe_partition();

                self.tickv.initialise(hash(tickv::MAIN_KEY))?;
                self.tickv.append_key(hash(key), buf)?;
            }
            return Err(e);
        }

        Ok(())
    }

    fn wipe_partition(&self) {
        let mut flash =
            esp_storage::FlashStorage::new(unsafe { self.flash_peripheral.clone_unchecked() });
        let mut address = self.offset as u32;
        let end = address + self.size as u32;

        while address < end {
            let next = (address + 1024).min(end);
            _ = embedded_storage::nor_flash::NorFlash::erase(&mut flash, address, next);
            address = next;
        }
    }

    pub async fn invalidate_key(&self, key: &[u8]) -> anyhow::Result<(), ErrorCode> {
        let _drop = self.semaphore.acquire(1).await.unwrap();
        self.tickv.invalidate_key(hash(key))?;
        self.tickv.garbage_collect()?;
        Ok(())
    }

    /// # Safety
    ///
    /// This doesn't check for semaphore!
    pub unsafe fn get_key_unchecked(
        &self,
        key: &[u8],
        buf: &mut [u8],
    ) -> anyhow::Result<(), ErrorCode> {
        self.tickv.get_key(hash(key), buf)?;
        Ok(())
    }

    /// # Safety
    ///
    /// This doesn't check for semaphore!
    pub unsafe fn append_key_unckeched(
        &self,
        key: &[u8],
        buf: &[u8],
    ) -> anyhow::Result<(), ErrorCode> {
        let res = self.tickv.append_key(hash(key), buf);
        if let Err(e) = res
            && e == ErrorCode::UnsupportedVersion
        {
            defmt::error!("Unsupported version while appending flash key... Wiping NVS partition!");

            self.wipe_partition();

            self.tickv.initialise(hash(tickv::MAIN_KEY))?;
            self.tickv.append_key(hash(key), buf)?;
        }

        Ok(())
    }

    /// # Safety
    ///
    /// This doesn't check for semaphore!
    pub unsafe fn invalidate_key_unchecked(&self, key: &[u8]) -> anyhow::Result<(), ErrorCode> {
        self.tickv.invalidate_key(hash(key))?;
        Ok(())
    }
}

impl Drop for Nvs {
    fn drop(&mut self) {
        NVS_INSTANCES.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
    }
}

impl Clone for Nvs {
    fn clone(&self) -> Self {
        NVS_INSTANCES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        Self {
            flash_peripheral: unsafe { self.flash_peripheral.clone_unchecked() },
            tickv: self.tickv.clone(),
            semaphore: self.semaphore.clone(),
            offset: self.offset,
            size: self.size,
        }
    }
}

pub struct NvsFlash {
    flash_offset: u32,
    flash: Mutex<CriticalSectionRawMutex, esp_storage::FlashStorage<'static>>,
}

impl NvsFlash {
    pub fn new(flash_offset: usize, flash: esp_hal::peripherals::FLASH<'static>) -> Self {
        Self {
            flash_offset: flash_offset as u32,
            flash: Mutex::new(esp_storage::FlashStorage::new(flash)),
        }
    }
}

impl FlashController<1024> for NvsFlash {
    fn read_region(
        &self,
        region_number: usize,
        buf: &mut [u8; 1024],
    ) -> Result<(), tickv::ErrorCode> {
        if let Ok(mut flash) = self.flash.try_lock() {
            let offset = region_number * 1024;
            flash
                .read(self.flash_offset + offset as u32, buf)
                .map_err(|_| tickv::ErrorCode::ReadFail)
        } else {
            Err(tickv::ErrorCode::ReadFail)
        }
    }

    fn write(&self, address: usize, buf: &[u8]) -> Result<(), tickv::ErrorCode> {
        if let Ok(mut flash) = self.flash.try_lock() {
            flash
                .write(self.flash_offset + address as u32, buf)
                .map_err(|_| tickv::ErrorCode::WriteFail)
        } else {
            Err(tickv::ErrorCode::WriteFail)
        }
    }

    fn erase_region(&self, region_number: usize) -> Result<(), tickv::ErrorCode> {
        if let Ok(mut flash) = self.flash.try_lock() {
            let offset = self.flash_offset + (region_number as u32 * 1024);
            embedded_storage::nor_flash::NorFlash::erase(&mut *flash, offset, offset + 1024)
                .map_err(|_| tickv::ErrorCode::EraseFail)
        } else {
            Err(tickv::ErrorCode::EraseFail)
        }
    }
}

pub fn hash(buf: &[u8]) -> u64 {
    crc32_be(!0xffffffff, buf) as u64
}
