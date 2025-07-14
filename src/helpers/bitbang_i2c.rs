use std::sync::{Arc, Mutex};

use esp_idf_svc::hal::{
    delay::Ets,
    gpio::{Gpio8, Gpio10, InputOutput, PinDriver, Pull},
    i2c::I2cDriver,
};

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
            SimpleBitBangError::Nack => embedded_hal::i2c::ErrorKind::NoAcknowledge(
                embedded_hal::i2c::NoAcknowledgeSource::Unknown,
            ),
            SimpleBitBangError::Timeout => embedded_hal::i2c::ErrorKind::ArbitrationLoss,
        }
    }
}

impl embedded_hal::i2c::ErrorType for SimpleBitBangI2cInstance {
    type Error = SimpleBitBangError;
}

impl SimpleBitBangI2cInstance {
    // Use timing based on VEML3328 datasheet - Standard Mode requirements
    const DELAY_LOW_US: u32 = 5; // t(LOW) >= 4.7μs
    const DELAY_HIGH_US: u32 = 5; // t(HIGH) >= 4.0μs
    const DELAY_SETUP_US: u32 = 1; // t(SUDAT) >= 250ns
    const DELAY_HOLD_US: u32 = 4; // t(HDDAT) <= 3450ns
    const DELAY_BUF_US: u32 = 5; // t(BUF) >= 4.7μs

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
        sda.set_pull(Pull::Up)
            .map_err(|_| SimpleBitBangError::GpioError)?;
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
        scl.set_pull(Pull::Up)
            .map_err(|_| SimpleBitBangError::GpioError)?;
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
        log::debug!("Writing I2C byte: 0x{byte:02X} (binary: {byte:08b})");
        // Send 8 bits, MSB first
        for i in 0..8 {
            let bit = (byte & (0x80 >> i)) != 0;
            log::debug!("  Bit {}: {}", i, if bit { 1 } else { 0 });
            self.write_bit(bit)?;
        }

        // Read ACK/NACK
        let ack = !self.read_bit()?; // ACK is low, NACK is high
        log::debug!(
            "Received ACK: {} ({})",
            ack,
            if ack { "ACK" } else { "NACK" }
        );
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
        log::debug!("Read I2C byte: 0x{byte:02X} (binary: {byte:08b}), sent ACK: {send_ack}");

        Ok(byte)
    }
}

impl embedded_hal::i2c::I2c for SimpleBitBangI2cInstance {
    fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        if read.is_empty() {
            return Ok(());
        }

        log::debug!(
            "I2C read from address 0x{:02X}, {} bytes",
            address,
            read.len()
        );

        self.start_condition()?;

        // Send address with read bit (1)
        let addr_byte = (address << 1) | 0x01;
        log::debug!("Sending address byte for read: 0x{addr_byte:02X}");
        if !self.write_byte(addr_byte)? {
            self.stop_condition()?;
            log::warn!("VEML3328 I2C NACK on address read: 0x{address:02X}");
            return Err(SimpleBitBangError::Nack);
        }

        // Read data bytes
        let read_len = read.len();
        for (i, byte) in read.iter_mut().enumerate() {
            let is_last = i == read_len - 1;
            *byte = self.read_byte(!is_last)?; // Send ACK for all but last byte
        }

        self.stop_condition()?;
        log::debug!("I2C read completed: {read:?}");
        Ok(())
    }

    fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        if write.is_empty() {
            return Ok(());
        }

        log::debug!(
            "I2C write to address 0x{:02X}, {} bytes: {:?}",
            address,
            write.len(),
            write
        );

        self.start_condition()?;

        // Send address with write bit (0)
        let addr_byte = (address << 1) & 0xFE;
        log::debug!("Sending address byte for write: 0x{addr_byte:02X}");
        if !self.write_byte(addr_byte)? {
            self.stop_condition()?;
            log::warn!("VEML3328 I2C NACK on address write: 0x{address:02X}");
            return Err(SimpleBitBangError::Nack);
        }

        // Send data bytes
        for &byte in write {
            if !self.write_byte(byte)? {
                self.stop_condition()?;
                log::warn!("VEML3328 I2C NACK on data write: 0x{byte:02X}");
                return Err(SimpleBitBangError::Nack);
            }
        }

        self.stop_condition()?;
        log::debug!("I2C write completed successfully");
        Ok(())
    }

    fn write_read(
        &mut self,
        address: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        log::debug!(
            "I2C write_read to address 0x{:02X}, write {} bytes: {:?}, read {} bytes",
            address,
            write.len(),
            write,
            read.len()
        );

        // Write phase
        if !write.is_empty() {
            self.start_condition()?;

            // Send address with write bit (0)
            let addr_byte = (address << 1) & 0xFE;
            log::debug!("Sending address byte for write: 0x{addr_byte:02X}");
            if !self.write_byte(addr_byte)? {
                self.stop_condition()?;
                log::warn!("VEML3328 I2C NACK on address write: 0x{address:02X}");
                return Err(SimpleBitBangError::Nack);
            }

            // Send data bytes
            for &byte in write {
                if !self.write_byte(byte)? {
                    self.stop_condition()?;
                    log::warn!("VEML3328 I2C NACK on data write: 0x{byte:02X}");
                    return Err(SimpleBitBangError::Nack);
                }
            }
        }

        // Read phase with repeated start
        if !read.is_empty() {
            self.start_condition()?; // Repeated start

            // Send address with read bit (1)
            let addr_byte = (address << 1) | 0x01;
            log::debug!("Sending address byte for read: 0x{addr_byte:02X}");
            if !self.write_byte(addr_byte)? {
                self.stop_condition()?;
                log::warn!("VEML3328 I2C NACK on address read: 0x{address:02X}");
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
        log::debug!("I2C write_read completed: read data: {read:?}");
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
        self.driver
            .lock()
            .unwrap()
            .read(address, read, 1000)
            .map_err(esp_idf_svc::hal::i2c::I2cError::other)
    }

    fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        self.driver
            .lock()
            .unwrap()
            .write(address, write, 1000)
            .map_err(esp_idf_svc::hal::i2c::I2cError::other)
    }

    fn write_read(
        &mut self,
        address: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        self.driver
            .lock()
            .unwrap()
            .write_read(address, write, read, 1000)
            .map_err(esp_idf_svc::hal::i2c::I2cError::other)
    }

    fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        self.driver
            .lock()
            .unwrap()
            .transaction(address, operations, 1000)
            .map_err(esp_idf_svc::hal::i2c::I2cError::other)
    }
}
