mod device_impl;

/// All possible errors in this crate
#[derive(Debug)]
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
    config: device_impl::Config,
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