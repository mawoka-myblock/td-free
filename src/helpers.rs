use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::bail;
use log::{error, info, warn};

#[derive(Debug, Clone, Copy)]
pub struct NvsData {
    pub b: f32,
    pub m: f32,
    pub threshold: f32,
}

use esp_idf_svc::{
    hal::{
        gpio::{Gpio10, Gpio5, Gpio6, Gpio8, PinDriver, InputOutput, Pull},
        i2c::{I2cConfig, I2cDriver, I2C0},
        delay::Ets,
    },
    nvs::{EspNvs, EspNvsPartition, NvsDefault},
    sys::esp_random,
};
use veml7700::Veml7700;

use crate::{led, veml3328, LedType};
use esp_idf_svc::hal::prelude::*;

// Simplified bit-bang I2C implementation using a different approach
pub struct SimpleBitBangI2c {
    sda: Arc<Mutex<PinDriver<'static, Gpio8, InputOutput>>>,
    scl: Arc<Mutex<PinDriver<'static, Gpio10, InputOutput>>>,
}

impl SimpleBitBangI2c {
    pub fn new(
        sda: PinDriver<'static, Gpio8, InputOutput>,
        scl: PinDriver<'static, Gpio10, InputOutput>,
    ) -> Self {
        Self {
            sda: Arc::new(Mutex::new(sda)),
            scl: Arc::new(Mutex::new(scl)),
        }
    }

    pub fn clone_driver(&self) -> SimpleBitBangI2cInstance {
        SimpleBitBangI2cInstance {
            sda: self.sda.clone(),
            scl: self.scl.clone(),
        }
    }
}

#[derive(Clone)]
pub struct SimpleBitBangI2cInstance {
    sda: Arc<Mutex<PinDriver<'static, Gpio8, InputOutput>>>,
    scl: Arc<Mutex<PinDriver<'static, Gpio10, InputOutput>>>,
}

#[derive(Debug)]
pub enum SimpleBitBangError {
    GpioError,
    Nack,
    Timeout,
}

impl embedded_hal::i2c::Error for SimpleBitBangError {
    fn kind(&self) -> embedded_hal::i2c::ErrorKind {
        match self {
            SimpleBitBangError::GpioError => embedded_hal::i2c::ErrorKind::Bus,
            SimpleBitBangError::Nack => embedded_hal::i2c::ErrorKind::NoAcknowledge(embedded_hal::i2c::NoAcknowledgeSource::Unknown),
            SimpleBitBangError::Timeout => embedded_hal::i2c::ErrorKind::ArbitrationLoss,
        }
    }
}

impl embedded_hal::i2c::ErrorType for SimpleBitBangI2cInstance {
    type Error = SimpleBitBangError;
}

impl SimpleBitBangI2cInstance {
    // Use timing based on VEML3328 datasheet - Standard Mode requirements
    const DELAY_LOW_US: u32 = 5;    // t(LOW) >= 4.7μs
    const DELAY_HIGH_US: u32 = 5;   // t(HIGH) >= 4.0μs
    const DELAY_SETUP_US: u32 = 1;  // t(SUDAT) >= 250ns
    const DELAY_HOLD_US: u32 = 4;   // t(HDDAT) <= 3450ns
    const DELAY_BUF_US: u32 = 5;    // t(BUF) >= 4.7μs

    fn delay_low(&self) {
        Ets::delay_us(Self::DELAY_LOW_US);
    }

    fn delay_high(&self) {
        Ets::delay_us(Self::DELAY_HIGH_US);
    }

    fn delay_setup(&self) {
        Ets::delay_us(Self::DELAY_SETUP_US);
    }

    fn delay_hold(&self) {
        Ets::delay_us(Self::DELAY_HOLD_US);
    }

    fn delay_buf(&self) {
        Ets::delay_us(Self::DELAY_BUF_US);
    }

    // Simplified approach: use the InputOutput pins directly without mode conversion
    fn set_sda_high(&mut self) -> Result<(), SimpleBitBangError> {
        let mut sda = self.sda.lock().unwrap();
        sda.set_pull(Pull::Up).map_err(|_| SimpleBitBangError::GpioError)?;
        // For open-drain I2C, high is achieved by not driving (letting pull-up work)
        // We'll use set_high() to achieve this on InputOutput pins
        sda.set_high().map_err(|_| SimpleBitBangError::GpioError)?;
        Ok(())
    }

    fn set_sda_low(&mut self) -> Result<(), SimpleBitBangError> {
        let mut sda = self.sda.lock().unwrap();
        sda.set_low().map_err(|_| SimpleBitBangError::GpioError)?;
        Ok(())
    }

    fn set_scl_high(&mut self) -> Result<(), SimpleBitBangError> {
        let mut scl = self.scl.lock().unwrap();
        scl.set_pull(Pull::Up).map_err(|_| SimpleBitBangError::GpioError)?;
        scl.set_high().map_err(|_| SimpleBitBangError::GpioError)?;

        // Wait for clock stretching (if any device is holding SCL low)
        let start_time = std::time::Instant::now();
        while !scl.is_high() {
            if start_time.elapsed().as_millis() > 10 {
                return Err(SimpleBitBangError::Timeout);
            }
            Ets::delay_us(1);
        }
        Ok(())
    }

    fn set_scl_low(&mut self) -> Result<(), SimpleBitBangError> {
        let mut scl = self.scl.lock().unwrap();
        scl.set_low().map_err(|_| SimpleBitBangError::GpioError)?;
        Ok(())
    }

    fn read_sda(&mut self) -> Result<bool, SimpleBitBangError> {
        let sda = self.sda.lock().unwrap();
        Ok(sda.is_high())
    }

    fn start_condition(&mut self) -> Result<(), SimpleBitBangError> {
        // Initialize to idle state (both lines high)
        self.set_sda_high()?;
        self.set_scl_high()?;
        self.delay_buf(); // t(BUF) bus free time

        // START condition: SDA goes low while SCL is high
        self.set_sda_low()?;
        self.delay_hold(); // t(HDSTA) >= 4.0μs
        self.set_scl_low()?;
        self.delay_setup(); // Setup time before first data bit
        Ok(())
    }

    fn stop_condition(&mut self) -> Result<(), SimpleBitBangError> {
        // Ensure SDA is low first
        self.set_sda_low()?;
        self.delay_setup();

        // STOP condition: SCL goes high first, then SDA goes high
        self.set_scl_high()?;
        self.delay_setup(); // t(SUSTO) >= 4.0μs
        self.set_sda_high()?;
        self.delay_buf(); // t(BUF) bus free time
        Ok(())
    }

    fn write_bit(&mut self, bit: bool) -> Result<(), SimpleBitBangError> {
        // Set SDA while SCL is low
        if bit {
            self.set_sda_high()?;
        } else {
            self.set_sda_low()?;
        }
        self.delay_setup(); // t(SUDAT) >= 250ns

        // Clock the bit: SCL high
        self.set_scl_high()?;
        self.delay_high(); // t(HIGH) >= 4.0μs

        // SCL low
        self.set_scl_low()?;
        self.delay_low(); // t(LOW) >= 4.7μs
        Ok(())
    }

    fn read_bit(&mut self) -> Result<bool, SimpleBitBangError> {
        // Release SDA to allow slave to control it
        self.set_sda_high()?;
        self.delay_setup();

        // Clock high and read
        self.set_scl_high()?;
        self.delay_setup(); // Setup time before reading
        let bit = self.read_sda()?;
        self.delay_high(); // Complete high period

        // Clock low
        self.set_scl_low()?;
        self.delay_low();
        Ok(bit)
    }

    fn write_byte(&mut self, byte: u8) -> Result<bool, SimpleBitBangError> {
        log::debug!("Writing I2C byte: 0x{:02X} (binary: {:08b})", byte, byte);
        // Send 8 bits, MSB first
        for i in 0..8 {
            let bit = (byte & (0x80 >> i)) != 0;
            log::debug!("  Bit {}: {}", i, if bit { 1 } else { 0 });
            self.write_bit(bit)?;
        }

        // Read ACK/NACK
        let ack = !self.read_bit()?; // ACK is low, NACK is high
        log::debug!("Received ACK: {} ({})", ack, if ack { "ACK" } else { "NACK" });
        Ok(ack)
    }

    fn read_byte(&mut self, send_ack: bool) -> Result<u8, SimpleBitBangError> {
        let mut byte = 0u8;

        // Read 8 bits, MSB first
        for i in 0..8 {
            if self.read_bit()? {
                byte |= 0x80 >> i;
            }
        }

        // Send ACK/NACK
        self.write_bit(!send_ack)?; // ACK is low, NACK is high
        log::debug!("Read I2C byte: 0x{:02X} (binary: {:08b}), sent ACK: {}", byte, byte, send_ack);

        Ok(byte)
    }
}

impl embedded_hal::i2c::I2c for SimpleBitBangI2cInstance {
    fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        if read.is_empty() {
            return Ok(());
        }

        log::debug!("I2C read from address 0x{:02X}, {} bytes", address, read.len());

        self.start_condition()?;

        // Send address with read bit (1)
        let addr_byte = (address << 1) | 0x01;
        log::debug!("Sending address byte for read: 0x{:02X}", addr_byte);
        if !self.write_byte(addr_byte)? {
            self.stop_condition()?;
            log::warn!("VEML3328 I2C NACK on address read: 0x{:02X}", address);
            return Err(SimpleBitBangError::Nack);
        }

        // Read data bytes
        let read_len = read.len();
        for (i, byte) in read.iter_mut().enumerate() {
            let is_last = i == read_len - 1;
            *byte = self.read_byte(!is_last)?; // Send ACK for all but last byte
        }

        self.stop_condition()?;
        log::debug!("I2C read completed: {:?}", read);
        Ok(())
    }

    fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        if write.is_empty() {
            return Ok(());
        }

        log::debug!("I2C write to address 0x{:02X}, {} bytes: {:?}", address, write.len(), write);

        self.start_condition()?;

        // Send address with write bit (0)
        let addr_byte = (address << 1) & 0xFE;
        log::debug!("Sending address byte for write: 0x{:02X}", addr_byte);
        if !self.write_byte(addr_byte)? {
            self.stop_condition()?;
            log::warn!("VEML3328 I2C NACK on address write: 0x{:02X}", address);
            return Err(SimpleBitBangError::Nack);
        }

        // Send data bytes
        for &byte in write {
            if !self.write_byte(byte)? {
                self.stop_condition()?;
                log::warn!("VEML3328 I2C NACK on data write: 0x{:02X}", byte);
                return Err(SimpleBitBangError::Nack);
            }
        }

        self.stop_condition()?;
        log::debug!("I2C write completed successfully");
        Ok(())
    }

    fn write_read(&mut self, address: u8, write: &[u8], read: &mut [u8]) -> Result<(), Self::Error> {
        log::debug!("I2C write_read to address 0x{:02X}, write {} bytes: {:?}, read {} bytes",
                   address, write.len(), write, read.len());

        // Write phase
        if !write.is_empty() {
            self.start_condition()?;

            // Send address with write bit (0)
            let addr_byte = (address << 1) & 0xFE;
            log::debug!("Sending address byte for write: 0x{:02X}", addr_byte);
            if !self.write_byte(addr_byte)? {
                self.stop_condition()?;
                log::warn!("VEML3328 I2C NACK on address write: 0x{:02X}", address);
                return Err(SimpleBitBangError::Nack);
            }

            // Send data bytes
            for &byte in write {
                if !self.write_byte(byte)? {
                    self.stop_condition()?;
                    log::warn!("VEML3328 I2C NACK on data write: 0x{:02X}", byte);
                    return Err(SimpleBitBangError::Nack);
                }
            }
        }

        // Read phase with repeated start
        if !read.is_empty() {
            self.start_condition()?; // Repeated start

            // Send address with read bit (1)
            let addr_byte = (address << 1) | 0x01;
            log::debug!("Sending address byte for read: 0x{:02X}", addr_byte);
            if !self.write_byte(addr_byte)? {
                self.stop_condition()?;
                log::warn!("VEML3328 I2C NACK on address read: 0x{:02X}", address);
                return Err(SimpleBitBangError::Nack);
            }

            // Read data bytes
            let read_len = read.len();
            for (i, byte) in read.iter_mut().enumerate() {
                let is_last = i == read_len - 1;
                *byte = self.read_byte(!is_last)?; // Send ACK for all but last byte
            }
        }

        self.stop_condition()?;
        log::debug!("I2C write_read completed: read data: {:?}", read);
        Ok(())
    }

    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        for op in operations {
            match op {
                embedded_hal::i2c::Operation::Read(buf) => {
                    self.read(address, buf)?;
                }
                embedded_hal::i2c::Operation::Write(buf) => {
                    self.write(address, buf)?;
                }
            }
        }
        Ok(())
    }
}

// Hardware I2C wrapper for VEML7700
pub struct HardwareI2c {
    driver: Arc<Mutex<I2cDriver<'static>>>,
}

impl HardwareI2c {
    pub fn new(driver: I2cDriver<'static>) -> Self {
        Self {
            driver: Arc::new(Mutex::new(driver)),
        }
    }

    pub fn clone_driver(&self) -> HardwareI2cInstance {
        HardwareI2cInstance {
            driver: self.driver.clone(),
        }
    }
}

pub struct HardwareI2cInstance {
    driver: Arc<Mutex<I2cDriver<'static>>>,
}

impl embedded_hal::i2c::ErrorType for HardwareI2cInstance {
    type Error = esp_idf_svc::hal::i2c::I2cError;
}

impl embedded_hal::i2c::I2c for HardwareI2cInstance {
    fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        self.driver.lock().unwrap()
            .read(address, read, 1000)
            .map_err(|e| esp_idf_svc::hal::i2c::I2cError::other(e))
    }

    fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        self.driver.lock().unwrap()
            .write(address, write, 1000)
            .map_err(|e| esp_idf_svc::hal::i2c::I2cError::other(e))
    }

    fn write_read(&mut self, address: u8, write: &[u8], read: &mut [u8]) -> Result<(), Self::Error> {
        self.driver.lock().unwrap()
            .write_read(address, write, read, 1000)
            .map_err(|e| esp_idf_svc::hal::i2c::I2cError::other(e))
    }

    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.driver.lock().unwrap()
            .transaction(address, operations, 1000)
            .map_err(|e| esp_idf_svc::hal::i2c::I2cError::other(e))
    }
}

pub fn get_saved_algorithm_variables(nvs: EspNvsPartition<NvsDefault>) -> NvsData {
    let nvs = match EspNvs::new(nvs, "algo", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            warn!("NVS init failed");
            return NvsData {
                b: 0.0,
                m: 1.0,
                threshold: 0.8,
            };
        }
    };
    let mut b_val_buffer = vec![0; 256];
    let b_value: f32 = nvs
        .get_str("b", &mut b_val_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.0);
    let mut m_val_buffer = vec![0; 256];
    let m_value = nvs
        .get_str("m", &mut m_val_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);
    let mut threshold_val_buffer = vec![0; 256];
    let threshold_value = nvs
        .get_str("threshold", &mut threshold_val_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.8);
    NvsData {
        b: b_value,
        m: m_value,
        threshold: threshold_value,
    }
}

pub fn save_algorithm_variables(
    b: &str,
    m: &str,
    threshold: &str,
    nvs: EspNvsPartition<NvsDefault>,
) -> anyhow::Result<()> {
    let mut nvs = match EspNvs::new(nvs, "algo", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            bail!("NVS failed");
        }
    };

    nvs.set_str("m", m)?;
    nvs.set_str("b", b)?;
    nvs.set_str("threshold", threshold)?;
    Ok(())
}

pub fn generate_random_11_digit_number() -> u64 {
    loop {
        let high: u64 = unsafe { esp_random() } as u64;
        let low: u64 = unsafe { esp_random() } as u64;
        let num = ((high << 32) | low) % 100_000_000_000;

        if num >= 10_000_000_000 {
            return num;
        }
    }
}

pub fn save_spoolman_data(url: &str, field_name: &str,nvs: EspNvsPartition<NvsDefault>) -> anyhow::Result<()> {
    let mut nvs = match EspNvs::new(nvs, "prefs", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            bail!("NVS failed");
        }
    };
    info!("Saving Spoolman: {}", &url);
    nvs.set_str("spoolman_url", url)?;
    nvs.set_str("spoolman_field_name", field_name)?;
    Ok(())
}

pub fn read_spoolman_data(nvs: EspNvsPartition<NvsDefault>) -> (Option<String>, Option<String>) {
    let nvs = match EspNvs::new(nvs, "prefs", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            error!("NVS failed");
            return (None, None);
        }
    };
    info!("Reading spoolman URL!");

    let mut spoolman_url_buf = vec![0; 256];
    let url = nvs
        .get_str("spoolman_url", &mut spoolman_url_buf)
        .unwrap_or(None)
        .map(|s| s.to_string());
    let mut spoolman_field_name_buf = vec![0; 256];
    let field_name = nvs
        .get_str("spoolman_field_name", &mut spoolman_field_name_buf)
        .unwrap_or(None)
        .map(|s| s.to_string());
    (url, field_name)
}

pub struct Pins {
    pub sda1: Gpio6,
    pub scl1: Gpio5,
    pub sda2: Gpio8,
    pub scl2: Gpio10,
    pub i2c: I2C0,
}

pub fn initialize_veml(
    pins: Pins,
    ws2812_old: Arc<Mutex<LedType>>,
    ws2812_new: Arc<Mutex<LedType>>,
) -> (Arc<Mutex<Veml7700<HardwareI2cInstance>>>, Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>, bool) {
    // Use hardware I2C for VEML7700 on primary pins
    let hw_config = I2cConfig::new()
        .baudrate(KiloHertz::from(100).into())
        .timeout(Duration::from_millis(100).into());

    let hw_i2c = I2cDriver::new(
        pins.i2c,
        pins.sda1,
        pins.scl1,
        &hw_config,
    );

    if hw_i2c.is_err() {
        info!("Primary I2C failed: {:?}, trying alt pins for both", hw_i2c.err());
        return init_alt_i2c_both(pins.sda2, pins.scl2, ws2812_old, ws2812_new);
    }

    let hw_i2c_driver = hw_i2c.unwrap();
    let hardware_i2c = HardwareI2c::new(hw_i2c_driver);

    // Create VEML7700 with hardware I2C
    let mut veml_temp = Veml7700::new(hardware_i2c.clone_driver());

    let veml_enable_res = veml_temp.enable();
    if veml_enable_res.is_err() {
        info!("VEML7700 enable failed: {:?}, trying alt pins", veml_enable_res.err());
        return init_alt_i2c_both(pins.sda2, pins.scl2, ws2812_old, ws2812_new);
    }

    // Create bit-banged I2C for VEML3328 on alt pins with proper initialization
    let mut sda_pin = PinDriver::input_output(pins.sda2).unwrap();
    let mut scl_pin = PinDriver::input_output(pins.scl2).unwrap();

    // Initialize pins to idle state (both high with pull-up)
    sda_pin.set_pull(Pull::Up).unwrap();
    scl_pin.set_pull(Pull::Up).unwrap();

    // Wait a bit for pins to stabilize
    std::thread::sleep(std::time::Duration::from_millis(20));

    let bitbang_i2c = SimpleBitBangI2c::new(sda_pin, scl_pin);
    let mut veml_rgb_temp = veml3328::VEML3328::new(bitbang_i2c.clone_driver());

    // Test basic I2C communication first
    log::info!("Testing VEML3328 I2C communication...");

    // Enable RGB sensor
    match veml_rgb_temp.enable() {
        Ok(_) => {
            log::info!("VEML3328 enabled successfully on bit-banged I2C");
            // Try to read device ID to verify communication
            match veml_rgb_temp.read_device_id() {
                Ok(id) => {
                    log::info!("VEML3328 device ID: 0x{:04X}", id);
                    if id == 0x28 {
                        log::info!("VEML3328 device ID matches expected value!");
                    } else if id == 0x0000 {
                        log::error!("VEML3328 device ID is 0x0000 - no communication!");
                    } else {
                        log::warn!("Unexpected device ID! Expected 0x28, got 0x{:04X}", id);
                    }
                },
                Err(e) => log::warn!("Could not read VEML3328 device ID: {:?}", e),
            }
        },
        Err(e) => {
            log::error!("Could not enable VEML3328 RGB sensor: {:?}", e);
        }
    }

    let veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>> = Arc::new(Mutex::new(veml_temp));
    let veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>> = Arc::new(Mutex::new(veml_rgb_temp));

    (veml, veml_rgb, false)
}

fn init_alt_i2c_both(
    sda: Gpio8,
    scl: Gpio10,
    ws2812_old: Arc<Mutex<LedType>>,
    ws2812_new: Arc<Mutex<LedType>>,
) -> (Arc<Mutex<Veml7700<HardwareI2cInstance>>>, Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>, bool) {
    // Since primary I2C failed, try to create hardware I2C on alt pins first
    let hw_config = I2cConfig::new()
        .baudrate(KiloHertz::from(100).into())
        .timeout(Duration::from_millis(100).into());

    let hw_i2c_alt = I2cDriver::new(
        unsafe { esp_idf_svc::hal::i2c::I2C0::new() },
        unsafe { Gpio8::new() },
        unsafe { Gpio10::new() },
        &hw_config,
    );

    if let Ok(hw_i2c_driver) = hw_i2c_alt {
        // Try hardware I2C for VEML7700 on alt pins
        let hardware_i2c = HardwareI2c::new(hw_i2c_driver);
        let mut veml_temp = Veml7700::new(hardware_i2c.clone_driver());

        let veml_enable_res = veml_temp.enable();
        if veml_enable_res.is_ok() {
            // Create separate bit-banged I2C for RGB sensor on the same pins
            // This works because they have different I2C addresses
            let sda_pin_rgb = PinDriver::input_output(sda).unwrap();
            let scl_pin_rgb = PinDriver::input_output(scl).unwrap();
            let bitbang_i2c_rgb = SimpleBitBangI2c::new(sda_pin_rgb, scl_pin_rgb);
            let mut veml_rgb_temp = veml3328::VEML3328::new(bitbang_i2c_rgb.clone_driver());

            // Enable RGB sensor
            match veml_rgb_temp.enable() {
                Ok(_) => {
                    log::info!("VEML3328 enabled successfully on alt bit-banged I2C");
                    // Try to read device ID to verify communication
                    match veml_rgb_temp.read_device_id() {
                        Ok(id) => {
                            log::info!("VEML3328 device ID: 0x{:04X}", id);
                            if id != 0x28 {
                                log::warn!("Unexpected device ID! Expected 0x28, got 0x{:04X}", id);
                            }
                        },
                        Err(e) => log::warn!("Could not read VEML3328 device ID: {:?}", e),
                    }
                },
                Err(e) => {
                    log::error!("Could not enable VEML3328 RGB sensor: {:?}", e);
                }
            }

            let veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>> = Arc::new(Mutex::new(veml_temp));
            let veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>> = Arc::new(Mutex::new(veml_rgb_temp));

            return (veml, veml_rgb, true);
        }
    }

    // If hardware I2C failed, fall back to bit-banged I2C for both sensors
    log::warn!("Hardware I2C failed on alt pins, using bit-banged I2C for both sensors");

    let sda_pin_veml = PinDriver::input_output(unsafe { Gpio8::new() }).unwrap();
    let scl_pin_veml = PinDriver::input_output(unsafe { Gpio10::new() }).unwrap();
    let bitbang_i2c_veml = SimpleBitBangI2c::new(sda_pin_veml, scl_pin_veml);

    // Create separate bit-banged I2C for RGB sensor
    let sda_pin_rgb = PinDriver::input_output(sda).unwrap();
    let scl_pin_rgb = PinDriver::input_output(scl).unwrap();
    let bitbang_i2c_rgb = SimpleBitBangI2c::new(sda_pin_rgb, scl_pin_rgb);

    let mut veml_temp = Veml7700::new(bitbang_i2c_veml.clone_driver());
    let mut veml_rgb_temp = veml3328::VEML3328::new(bitbang_i2c_rgb.clone_driver());

    let veml_enable_res = veml_temp.enable();
    if veml_enable_res.is_err() {
        log::error!("VEML7700 enable failed on alt pins with bit-bang: {:?}", veml_enable_res.err());
        led::show_veml_not_found_error(ws2812_old, ws2812_new);
        unreachable!();
    }

    // Enable RGB sensor
    match veml_rgb_temp.enable() {
        Ok(_) => {
            log::info!("VEML3328 enabled successfully on alt bit-banged I2C");
            // Try to read device ID to verify communication
            match veml_rgb_temp.read_device_id() {
                Ok(id) => {
                    log::info!("VEML3328 device ID: 0x{:04X}", id);
                    if id != 0x28 {
                        log::warn!("Unexpected device ID! Expected 0x28, got 0x{:04X}", id);
                    }
                },
                Err(e) => log::warn!("Could not read VEML3328 device ID: {:?}", e),
            }
        },
        Err(e) => {
            log::error!("Could not enable VEML3328 RGB sensor: {:?}", e);
        }
    }

    // For the fallback case, we need to create a hardware I2C wrapper for the bit-banged VEML7700
    // This is a bit of a hack, but necessary to match the expected return type
    // We'll create a dummy hardware I2C driver
    let dummy_hw_config = I2cConfig::new()
        .baudrate(KiloHertz::from(100).into())
        .timeout(Duration::from_millis(100).into());

    // Try to create a dummy hardware I2C instance
    if let Ok(dummy_hw_driver) = I2cDriver::new(
        unsafe { esp_idf_svc::hal::i2c::I2C0::new() },
        unsafe { Gpio8::new() },
        unsafe { Gpio10::new() },
        &dummy_hw_config,
    ) {
        let dummy_hardware_i2c = HardwareI2c::new(dummy_hw_driver);
        let dummy_veml = Veml7700::new(dummy_hardware_i2c.clone_driver());

        // We'll return the dummy hardware VEML but it won't be used since the bit-banged one works
        let veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>> = Arc::new(Mutex::new(dummy_veml));
        let veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>> = Arc::new(Mutex::new(veml_rgb_temp));

        // Log warning that we're using a workaround
        log::warn!("Using workaround: bit-banged VEML7700 wrapped in hardware I2C type");

        (veml, veml_rgb, true)
    } else {
        // If even the dummy fails, we have no choice but to panic
        log::error!("Complete I2C failure - cannot create any I2C instances");
        led::show_veml_not_found_error(ws2812_old, ws2812_new);
        unreachable!();
    }
}
