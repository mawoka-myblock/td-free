use embassy_time::Timer;
use esp_hal::gpio::{Flex, InputConfig, OutputConfig};

#[derive(Debug)]
pub enum BitBangI2cError {
    GpioError,
    Nack,
    Timeout,
}

impl embedded_hal::i2c::Error for BitBangI2cError {
    fn kind(&self) -> embedded_hal::i2c::ErrorKind {
        match self {
            BitBangI2cError::GpioError => embedded_hal::i2c::ErrorKind::Bus,
            BitBangI2cError::Nack => embedded_hal::i2c::ErrorKind::NoAcknowledge(
                embedded_hal::i2c::NoAcknowledgeSource::Unknown,
            ),
            BitBangI2cError::Timeout => embedded_hal::i2c::ErrorKind::ArbitrationLoss,
        }
    }
}

/// Async bit-bang I2C driver for esp-hal (no_std).
///
/// SDA and SCL pins must be configured as open-drain outputs with pull-ups enabled.
/// Use `esp_hal::gpio::OutputOpenDrain` if available, or configure pull-ups externally.
pub struct BitBangI2c<'d> {
    sda: &'d mut esp_hal::gpio::Flex<'d>,
    scl: &'d mut esp_hal::gpio::Flex<'d>,
}
impl<'d> BitBangI2c<'d> {
    // Standard Mode timings (I2C spec + VEML3328 datasheet)
    const DELAY_LOW_US: u64 = 5; // t(LOW)  >= 4.7 μs
    const DELAY_HIGH_US: u64 = 5; // t(HIGH) >= 4.0 μs
    const DELAY_SETUP_US: u64 = 1; // t(SUDAT) >= 250 ns
    const DELAY_HOLD_US: u64 = 4; // t(HDDAT) <= 3450 ns
    const DELAY_BUF_US: u64 = 5; // t(BUF)  >= 4.7 μs

    /// Create a new bit-bang I2C driver.
    ///
    /// Both pins must already be configured as open-drain with pull-ups.
    pub fn new(sda: &'d mut Flex<'d>, scl: &'d mut Flex<'d>) -> Self {
        sda.set_input_enable(true);
        sda.apply_input_config(&InputConfig::default().with_pull(esp_hal::gpio::Pull::None));
        sda.apply_output_config(
            &OutputConfig::default().with_drive_mode(esp_hal::gpio::DriveMode::OpenDrain),
        );

        scl.set_input_enable(true);
        scl.apply_input_config(&InputConfig::default().with_pull(esp_hal::gpio::Pull::None));
        scl.apply_output_config(
            &OutputConfig::default().with_drive_mode(esp_hal::gpio::DriveMode::OpenDrain),
        );
        Self { sda, scl }
    }

    // --- Pin helpers ---------------------------------------------------------

    #[inline]
    fn sda_high(&mut self) {
        // Release SDA — pull-up takes it high (open-drain)
        self.sda.set_high();
    }

    #[inline]
    fn sda_low(&mut self) {
        self.sda.set_low();
    }

    #[inline]
    fn scl_high(&mut self) {
        // Release SCL — pull-up takes it high (open-drain)
        self.scl.set_high();
    }

    #[inline]
    fn scl_low(&mut self) {
        self.scl.set_low();
    }

    #[inline]
    fn read_sda(&self) -> bool {
        self.sda.is_high()
    }

    #[inline]
    fn read_scl(&self) -> bool {
        self.scl.is_high()
    }

    // --- Delays --------------------------------------------------------------

    #[inline]
    async fn delay_low(&self) {
        Timer::after_micros(Self::DELAY_LOW_US).await;
    }

    #[inline]
    async fn delay_high(&self) {
        Timer::after_micros(Self::DELAY_HIGH_US).await;
    }

    #[inline]
    async fn delay_setup(&self) {
        Timer::after_micros(Self::DELAY_SETUP_US).await;
    }

    #[inline]
    async fn delay_hold(&self) {
        Timer::after_micros(Self::DELAY_HOLD_US).await;
    }

    #[inline]
    async fn delay_buf(&self) {
        Timer::after_micros(Self::DELAY_BUF_US).await;
    }

    // --- Clock stretching ----------------------------------------------------

    /// Wait until SCL is actually high (handles clock stretching).
    /// Times out after ~10 ms using 1 μs async polls.
    async fn wait_scl_high(&self) -> Result<(), BitBangI2cError> {
        // 10 ms / 1 μs = 10_000 iterations max
        for _ in 0..10_000u32 {
            if self.read_scl() {
                return Ok(());
            }
            Timer::after_micros(1).await;
        }
        Err(BitBangI2cError::Timeout)
    }

    // --- Bus conditions ------------------------------------------------------

    async fn start_condition(&mut self) -> Result<(), BitBangI2cError> {
        // Idle: both lines high
        self.sda_high();
        self.scl_high();
        self.wait_scl_high().await?;
        self.delay_buf().await;

        // START: SDA falls while SCL is high
        self.sda_low();
        self.delay_hold().await; // t(HDSTA) >= 4.0 μs
        self.scl_low();
        self.delay_setup().await;
        Ok(())
    }

    async fn repeated_start_condition(&mut self) -> Result<(), BitBangI2cError> {
        // Re-assert SDA high before the repeated start
        self.sda_high();
        self.delay_setup().await;
        self.scl_high();
        self.wait_scl_high().await?;
        self.delay_setup().await; // t(SUSTA) >= 4.7 μs

        // START: SDA falls while SCL is high
        self.sda_low();
        self.delay_hold().await;
        self.scl_low();
        self.delay_setup().await;
        Ok(())
    }

    async fn stop_condition(&mut self) -> Result<(), BitBangI2cError> {
        self.sda_low();
        self.delay_setup().await;

        // STOP: SDA rises while SCL is high
        self.scl_high();
        self.wait_scl_high().await?;
        self.delay_setup().await; // t(SUSTO) >= 4.0 μs
        self.sda_high();
        self.delay_buf().await;
        Ok(())
    }

    // --- Bit I/O -------------------------------------------------------------

    async fn write_bit(&mut self, bit: bool) -> Result<(), BitBangI2cError> {
        if bit {
            self.sda_high();
        } else {
            self.sda_low();
        }
        self.delay_setup().await; // t(SUDAT) >= 250 ns

        self.scl_high();
        self.wait_scl_high().await?;
        self.delay_high().await;

        self.scl_low();
        self.delay_low().await;
        Ok(())
    }

    async fn read_bit(&mut self) -> Result<bool, BitBangI2cError> {
        self.sda_high(); // Release SDA
        self.delay_setup().await;

        self.scl_high();
        self.wait_scl_high().await?;
        self.delay_setup().await;
        let bit = self.read_sda();
        self.delay_high().await;

        self.scl_low();
        self.delay_low().await;
        Ok(bit)
    }

    // --- Byte I/O ------------------------------------------------------------

    /// Write one byte, return `true` on ACK, `false` on NACK.
    async fn write_byte(&mut self, byte: u8) -> Result<bool, BitBangI2cError> {
        for i in 0..8u8 {
            self.write_bit((byte & (0x80 >> i)) != 0).await?;
        }
        // ACK = slave pulls SDA low
        let nack = self.read_bit().await?;
        Ok(!nack)
    }

    /// Read one byte and send ACK (`send_ack = true`) or NACK (`send_ack = false`).
    async fn read_byte(&mut self, send_ack: bool) -> Result<u8, BitBangI2cError> {
        let mut byte = 0u8;
        for i in 0..8u8 {
            if self.read_bit().await? {
                byte |= 0x80 >> i;
            }
        }
        // ACK = drive low; NACK = release high
        self.write_bit(!send_ack).await?;
        Ok(byte)
    }

    // --- Address phase -------------------------------------------------------

    async fn send_address_write(&mut self, address: u8) -> Result<(), BitBangI2cError> {
        let addr_byte = (address << 1) & 0xFE;
        if !self.write_byte(addr_byte).await? {
            self.stop_condition().await?;
            return Err(BitBangI2cError::Nack);
        }
        Ok(())
    }

    async fn send_address_read(&mut self, address: u8) -> Result<(), BitBangI2cError> {
        let addr_byte = (address << 1) | 0x01;
        if !self.write_byte(addr_byte).await? {
            self.stop_condition().await?;
            return Err(BitBangI2cError::Nack);
        }
        Ok(())
    }
}

// --- embedded-hal-async I2c impl --------------------------------------------

impl<'d> embedded_hal::i2c::ErrorType for BitBangI2c<'d> {
    type Error = BitBangI2cError;
}

impl<'d> embedded_hal_async::i2c::I2c for BitBangI2c<'d> {
    async fn read(&mut self, address: u8, read: &mut [u8]) -> Result<(), Self::Error> {
        if read.is_empty() {
            return Ok(());
        }

        self.start_condition().await?;
        self.send_address_read(address).await?;

        let last = read.len() - 1;
        for (i, byte) in read.iter_mut().enumerate() {
            *byte = self.read_byte(i != last).await?;
        }

        self.stop_condition().await
    }

    async fn write(&mut self, address: u8, write: &[u8]) -> Result<(), Self::Error> {
        if write.is_empty() {
            return Ok(());
        }

        self.start_condition().await?;
        self.send_address_write(address).await?;

        for &byte in write {
            if !self.write_byte(byte).await? {
                self.stop_condition().await?;
                return Err(BitBangI2cError::Nack);
            }
        }

        self.stop_condition().await
    }

    async fn write_read(
        &mut self,
        address: u8,
        write: &[u8],
        read: &mut [u8],
    ) -> Result<(), Self::Error> {
        if !write.is_empty() {
            self.start_condition().await?;
            self.send_address_write(address).await?;

            for &byte in write {
                if !self.write_byte(byte).await? {
                    self.stop_condition().await?;
                    return Err(BitBangI2cError::Nack);
                }
            }
        }

        if !read.is_empty() {
            self.repeated_start_condition().await?;
            self.send_address_read(address).await?;

            let last = read.len() - 1;
            for (i, byte) in read.iter_mut().enumerate() {
                *byte = self.read_byte(i != last).await?;
            }
        }

        self.stop_condition().await
    }

    async fn transaction(
        &mut self,
        address: u8,
        operations: &mut [embedded_hal::i2c::Operation<'_>],
    ) -> Result<(), Self::Error> {
        use embedded_hal::i2c::Operation;

        let mut iter = operations.iter_mut().peekable();

        while let Some(op) = iter.next() {
            let is_first = true; // start is always issued before first op
            let _ = is_first;

            match op {
                Operation::Write(buf) => {
                    self.start_condition().await?;
                    self.send_address_write(address).await?;
                    for &byte in buf.iter() {
                        if !self.write_byte(byte).await? {
                            self.stop_condition().await?;
                            return Err(BitBangI2cError::Nack);
                        }
                    }
                }
                Operation::Read(buf) => {
                    self.start_condition().await?;
                    self.send_address_read(address).await?;
                    let last = buf.len().saturating_sub(1);
                    for (i, byte) in buf.iter_mut().enumerate() {
                        *byte = self.read_byte(i != last).await?;
                    }
                }
            }

            // Issue STOP only after the final operation
            if iter.peek().is_none() {
                self.stop_condition().await?;
            }
        }

        Ok(())
    }
}
