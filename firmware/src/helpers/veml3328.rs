#![allow(dead_code)]

use defmt::Format;
use embassy_time::Timer;
use embedded_hal::i2c::I2c;

/// All possible errors in this crate
#[derive(Debug, Format)]
pub enum Error<E> {
    /// I²C bus error
    I2C(E),
}
impl<E> From<E> for Error<E> {
    fn from(other: E) -> Self {
        Error::I2C(other)
    }
}
const DEVICE_ADDRESS: u8 = 0x10;

/// VEML3328 device driver.
#[derive(Debug)]
pub struct VEML3328<I2C> {
    /// The concrete I²C device implementation.
    i2c: I2C,
    config: Config,
    // gain: Gain,
    // it: IntegrationTime,
}

/// Integration time
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IntegrationTime {
    /// 25 ms
    _25ms,
    /// 50 ms
    _50ms,
    /// 100 ms
    _100ms,
    /// 200 ms
    _200ms,
    /// 400 ms
    _400ms,
    /// 800 ms
    _800ms,
}

impl IntegrationTime {
    /// Return the integration time in milliseconds
    pub fn as_ms(&self) -> u16 {
        match self {
            IntegrationTime::_25ms => 25,
            IntegrationTime::_50ms => 50,
            IntegrationTime::_100ms => 100,
            IntegrationTime::_200ms => 200,
            IntegrationTime::_400ms => 400,
            IntegrationTime::_800ms => 800,
        }
    }

    /// Return the integration time in microseconds
    pub fn as_us(&self) -> u32 {
        (self.as_ms() as u32) * 1000
    }
}

/// Gain
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gain {
    /// 1/8
    OneEighth,
    /// 1/4
    OneQuarter,
    /// 1 (default)
    One,
    /// 2
    Two,
}

/// Fault count
///
/// Number of consecutive fault events necessary to trigger interrupt.
/// This is referred to as "persistence" in the documentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FaultCount {
    /// One (default)
    One,
    /// Two
    Two,
    /// Four
    Four,
    /// Eight
    Eight,
}

/// Power-saving mode
///
/// This combined with the integration time determines the repetition rate
/// and the power consumption of the device.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PowerSavingMode {
    /// One
    One,
    /// Two
    Two,
    /// Three
    Three,
    /// Four
    Four,
}

/// Interrupt status
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterruptStatus {
    /// Whether the low threshold was exceeded consecutively as many times
    /// as configured as fault count.
    pub was_too_low: bool,
    /// Whether the high threshold was exceeded consecutively as many times
    /// as configured as fault count.
    pub was_too_high: bool,
}

struct Register;
impl Register {
    const CONFIG: u8 = 0x00;
    const C_DATA: u8 = 0x04;
    const R_DATA: u8 = 0x05;
    const G_DATA: u8 = 0x06;
    const B_DATA: u8 = 0x07;
    const IR_DATA: u8 = 0x08;
    const ID_DATA: u8 = 0x0C;
}

mod bits {
    /// SD0 — chip shutdown (bit 0)
    pub const SD0: u16 = 1 << 0;
    /// SD1 — channel shutdown (bit 15)
    pub const SD1: u16 = 1 << 15;
    /// Gain bits [12:11]
    pub const GAIN_MASK: u16 = 0b11 << 11;
    pub const GAIN_ONE_EIGHTH: u16 = 0b11 << 11;
    pub const GAIN_ONE_QUARTER: u16 = 0b10 << 11;
    pub const GAIN_ONE: u16 = 0b00 << 11;
    pub const GAIN_TWO: u16 = 0b01 << 11;
    /// Integration time bits [6:4]
    pub const IT_MASK: u16 = 0b111 << 4;
    pub const IT_25MS: u16 = 0b1100 << 4;
    pub const IT_50MS: u16 = 0b1000 << 4;
    pub const IT_100MS: u16 = 0b0000 << 4;
    pub const IT_200MS: u16 = 0b0001 << 4;
    pub const IT_400MS: u16 = 0b0010 << 4;
    pub const IT_800MS: u16 = 0b0011 << 4;
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub(crate) bits: u16,
}

impl Config {
    pub(crate) fn new() -> Self {
        // Power-on defaults: SD0=0, SD1=0, gain=1x, IT=50ms
        Config { bits: 0x0000 }
    }

    fn with_high(self, mask: u16) -> Self {
        Config {
            bits: self.bits | mask,
        }
    }

    fn with_low(self, mask: u16) -> Self {
        Config {
            bits: self.bits & !mask,
        }
    }

    fn set_bits(self, mask: u16, value: u16) -> Self {
        Config {
            bits: (self.bits & !mask) | (value & mask),
        }
    }
}

impl<I2C> VEML3328<I2C>
where
    I2C: I2c,
{
    pub fn new(i2c: I2C) -> Self {
        VEML3328 {
            i2c,
            config: Config::new(),
        }
    }

    pub fn destroy(self) -> I2C {
        self.i2c
    }

    /// Power on the sensor and apply default configuration.
    pub fn enable(&mut self) -> Result<(), Error<I2C::Error>> {
        // Verify communication
        let id = self.read_register(Register::ID_DATA)?;
        if id == 0x0000 {
            // Force an I2C error by issuing a dummy write to an invalid address
            self.i2c.write(0xFF, &[]).map_err(Error::I2C)?;
        }

        // Clear shutdown bits, set 100 ms integration time, gain 1x
        let config = Config::new()
            .with_low(bits::SD0 | bits::SD1)
            .set_bits(bits::IT_MASK, bits::IT_100MS)
            .set_bits(bits::GAIN_MASK, bits::GAIN_ONE);

        self.set_config(config)?;
        Timer::after_millis(150);
        Ok(())
    }

    /// Power off the sensor.
    pub fn disable(&mut self) -> Result<(), Error<I2C::Error>> {
        let config = self.config.with_high(bits::SD0);
        self.set_config(config)
    }

    /// Set the gain.
    pub fn set_gain(&mut self, gain: Gain) -> Result<(), Error<I2C::Error>> {
        let gain_bits = match gain {
            Gain::OneEighth => bits::GAIN_ONE_EIGHTH,
            Gain::OneQuarter => bits::GAIN_ONE_QUARTER,
            Gain::One => bits::GAIN_ONE,
            Gain::Two => bits::GAIN_TWO,
        };
        let config = self.config.set_bits(bits::GAIN_MASK, gain_bits);
        self.set_config(config)
    }

    /// Set the integration time.
    pub fn set_integration_time(&mut self, it: IntegrationTime) -> Result<(), Error<I2C::Error>> {
        let it_bits = match it {
            IntegrationTime::_25ms => bits::IT_25MS,
            IntegrationTime::_50ms => bits::IT_50MS,
            IntegrationTime::_100ms => bits::IT_100MS,
            IntegrationTime::_200ms => bits::IT_200MS,
            IntegrationTime::_400ms => bits::IT_400MS,
            IntegrationTime::_800ms => bits::IT_800MS,
        };
        let config = self.config.set_bits(bits::IT_MASK, it_bits);
        self.set_config(config)
    }

    pub fn read_red(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.read_register(Register::R_DATA)
    }

    pub fn read_green(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.read_register(Register::G_DATA)
    }

    pub fn read_blue(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.read_register(Register::B_DATA)
    }

    pub fn read_clear(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.read_register(Register::C_DATA)
    }

    pub fn read_ir(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.read_register(Register::IR_DATA)
    }

    pub fn read_device_id(&mut self) -> Result<u16, Error<I2C::Error>> {
        self.read_register(Register::ID_DATA)
    }

    /// Read all 16 registers and return them as an array (debug helper).
    pub fn read_all_registers(&mut self) -> Result<[u16; 16], Error<I2C::Error>> {
        let mut registers = [0u16; 16];
        for i in 0u8..16 {
            registers[i as usize] = self.read_register(i).unwrap_or(0);
        }
        Ok(registers)
    }

    // --- Internal helpers ----------------------------------------------------

    fn set_config(&mut self, config: Config) -> Result<(), Error<I2C::Error>> {
        self.write_register(Register::CONFIG, config.bits)?;
        self.config = config;
        Ok(())
    }

    fn write_register(&mut self, register: u8, value: u16) -> Result<(), Error<I2C::Error>> {
        let data = [register, value as u8, (value >> 8) as u8];
        self.i2c.write(DEVICE_ADDRESS, &data).map_err(Error::I2C)
    }

    fn read_register(&mut self, register: u8) -> Result<u16, Error<I2C::Error>> {
        let mut buf = [0u8; 2];
        self.i2c
            .write_read(DEVICE_ADDRESS, &[register], &mut buf)
            .map_err(Error::I2C)?;
        // Datasheet: buf[0] = LSB, buf[1] = MSB
        Ok(u16::from(buf[0]) | (u16::from(buf[1]) << 8))
    }
}
