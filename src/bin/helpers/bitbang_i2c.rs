use critical_section::Mutex;
use embassy_time::{Duration, Instant, Timer};
use esp_hal::peripherals::{GPIO10, GPIO8};

static I2C_INSTANCE: Mutex<Option<SimpleBitBangI2c>> = Mutex::new(None);

pub struct SimpleBitBangI2c {
    sda: GPIO8<'static>,
    scl: GPIO10<'static>,
}

impl SimpleBitBangI2c {
    pub fn new(sda: GPIO8, scl: GPIO10) -> Self {
        Self { sda, scl }
    }

    // Initialize the global instance
    pub fn init_global(sda: GPIO8, scl: GPIO10) {
        critical_section::with(|cs| {
            *I2C_INSTANCE.borrow_ref_mut(cs) = Some(Self::new(sda, scl));
        });
    }

    // Execute a function with access to the global I2C instance
    pub fn with_global<F, R>(f: F) -> Option<R>
    where
        F: FnOnce(&mut SimpleBitBangI2c) -> R,
    {
        critical_section::with(|cs| {
            if let Some(ref mut i2c) = *I2C_INSTANCE.borrow_ref_mut(cs) {
                Some(f(i2c))
            } else {
                None
            }
        })
    }

    // Use timing based on VEML3328 datasheet - Standard Mode requirements
    const DELAY_LOW_US: u64 = 5; // t(LOW) >= 4.7μs
    const DELAY_HIGH_US: u64 = 5; // t(HIGH) >= 4.0μs
    const DELAY_SETUP_US: u64 = 1; // t(SUDAT) >= 250ns
    const DELAY_HOLD_US: u64 = 4; // t(HDDAT) <= 3450ns
    const DELAY_BUF_US: u64 = 5; // t(BUF) >= 4.7μs

    async fn delay_low(&self) {
        Timer::after(Duration::from_micros(Self::DELAY_LOW_US)).await;
    }

    async fn delay_high(&self) {
        Timer::after(Duration::from_micros(Self::DELAY_HIGH_US)).await;
    }

    async fn delay_setup(&self) {
        Timer::after(Duration::from_micros(Self::DELAY_SETUP_US)).await;
    }

    async fn delay_hold(&self) {
        Timer::after(Duration::from_micros(Self::DELAY_HOLD_US)).await;
    }

    async fn delay_buf(&self) {
        Timer::after(Duration::from_micros(Self::DELAY_BUF_US)).await;
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

    async fn set_scl_high(&mut self) -> Result<(), SimpleBitBangError> {
        let mut scl = self.scl.lock().unwrap();
        scl.set_pull(Pull::Up)
            .map_err(|_| SimpleBitBangError::GpioError)?;
        scl.set_high().map_err(|_| SimpleBitBangError::GpioError)?;

        // Wait for clock stretching (if any device is holding SCL low)
        let start_time = Instant::now();
        while !scl.is_high() {
            if start_time.elapsed().as_millis() > 10 {
                return Err(SimpleBitBangError::Timeout);
            }
            Timer::after(Duration::from_micros(1)).await;
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
        log::debug!(
            "Read I2C byte: 0x{:02X} (binary: {:08b}), sent ACK: {}",
            byte,
            byte,
            send_ack
        );

        Ok(byte)
    }
}
