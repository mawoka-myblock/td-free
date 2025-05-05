use embedded_hal::i2c::{ErrorType, I2c, SevenBitAddress};
use crate::veml3328::{Error, VEML3328};

const DEVICE_ADDRESS: u8 = 0x10;

struct Register;
impl Register {
    const CONFIG: u8 = 0x00;
    const R_DATA: u8 = 0x05;
    const G_DATA: u8 = 0x06;
    const B_DATA: u8 = 0x07;
    const C_DATA: u8 = 0x04;
    const IR_DATA: u8 = 0x08;
}

#[derive(Debug, Clone, Copy)]
pub struct Config {
    bits: u16,
}

impl Config {
    fn new() -> Self {
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
}


impl<I2C> VEML3328<I2C>
where
    I2C: I2c<SevenBitAddress>,
    I2C::Error: Into<Error<I2C::Error>>,
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

    pub fn enable(&mut self) -> Result<(), Error<I2C::Error>> {
        let config = self.config.with_low(0x01);
        self.set_config(config)
    }

    pub fn disable(&mut self) -> Result<(), Error<I2C::Error>> {
        let config = self.config.with_high(0x01);
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

    fn set_config(&mut self, config: Config) -> Result<(), Error<I2C::Error>> {
        self.write_register(Register::CONFIG, config.bits)?;
        self.config = config;
        Ok(())
    }

    fn write_register(
        &mut self,
        register: u8,
        value: u16,
    ) -> Result<(), <I2C as ErrorType>::Error> {
        self.i2c
            .write(DEVICE_ADDRESS, &[register, value as u8, (value >> 8) as u8])
    }

    fn read_register(&mut self, register: u8) -> Result<u16, Error<I2C::Error>> {
        let mut data = [0; 2];
        self.i2c
            .write_read(DEVICE_ADDRESS, &[register], &mut data)
            .map_err(Error::I2C)
            .and(Ok(u16::from(data[0]) | u16::from(data[1]) << 8))
    }
}
