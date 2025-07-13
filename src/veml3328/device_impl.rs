use crate::veml3328::{Error, VEML3328};

const DEVICE_ADDRESS: u8 = 0x10;

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

#[derive(Debug, Clone, Copy)]
pub struct Config {
    bits: u16,
}

impl Config {
    fn new() -> Self {
        // Start with proper default configuration
        // Bit 0 (SD0) = 0 (power on)
        // Bit 15 (SD1) = 0 (power on)
        // Integration time = 50ms (default)
        // Gain = 1 (default)
        Config { bits: 0x0000 }
    }

    fn with_high(self, mask: u16) -> Self {
        Config {
            bits: self.bits | mask,
        }
    }
    #[allow(dead_code)]
    fn with_low(self, mask: u16) -> Self {
        Config {
            bits: self.bits & !mask,
        }
    }
}

impl<I2C> VEML3328<I2C>
where
    I2C: embedded_hal::i2c::I2c,
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
        log::info!("Starting VEML3328 enable sequence...");

        // First, try a simple register read to test communication
        match self.read_register(Register::ID_DATA) {
            Ok(id) => {
                log::info!("VEML3328 Device ID before enable: 0x{id:04X}");
                if id == 0x0000 {
                    log::error!("Device ID is 0x0000 - I2C communication failed!");
                    // Return a generic I2C error - we can't create the specific error type
                    // without knowing the concrete I2C implementation
                    return Err(Error::I2C(self.i2c.write(0xFF, &[]).unwrap_err()));
                }
            }
            Err(e) => {
                log::error!("Failed to read device ID during enable: {e:?}");
                return Err(e);
            }
        }

        // Try to read current config
        let current_config = match self.read_register(Register::CONFIG) {
            Ok(cfg) => {
                log::info!("Current config register: 0x{cfg:04X}");
                cfg
            }
            Err(e) => {
                log::warn!("Could not read current config: {e:?}");
                0x8001 // Default to shutdown state (both SD0 and SD1 set)
            }
        };

        // Configure for optimal color measurement
        // Set integration time to 100ms (bits 6-4 = 010) for better accuracy
        // Keep gain at 1x (bits 12-11 = 00) for normal sensitivity
        // Clear shutdown bits (bits 15 and 0)
        let new_config_bits = (current_config & !0x8071) | 0x0020; // Clear shutdown and set 100ms integration time
        let config = Config {
            bits: new_config_bits,
        };

        log::info!(
            "Writing optimized config for color measurement: 0x{:04X}",
            config.bits
        );
        self.set_config(config)?;

        // Add longer delay for sensor to stabilize with new settings
        std::thread::sleep(std::time::Duration::from_millis(150));

        // Verify configuration was written
        let read_config = self.read_register(Register::CONFIG)?;
        log::info!("VEML3328 Config after enable: 0x{read_config:04X}");

        // Verify device ID again after configuration
        let final_id = self.read_register(Register::ID_DATA)?;
        log::info!("VEML3328 Device ID after enable: 0x{final_id:04X}");

        // Take a few test readings to verify sensor is working
        std::thread::sleep(std::time::Duration::from_millis(110)); // Wait for integration time
        match (
            self.read_register(Register::R_DATA),
            self.read_register(Register::G_DATA),
            self.read_register(Register::B_DATA),
        ) {
            (Ok(r), Ok(g), Ok(b)) => {
                log::info!("Initial color readings after enable: R={r}, G={g}, B={b}",);
                if r == 0 && g == 0 && b == 0 {
                    log::warn!(
                        "All color readings are zero - sensor might not be working properly"
                    );
                }
            }
            _ => log::warn!("Could not read initial color values after enable"),
        }

        Ok(())
    }

    pub fn disable(&mut self) -> Result<(), Error<I2C::Error>> {
        // Power off by setting SD0 (bit 0)
        let config = self.config.with_high(0x0001);
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

    fn set_config(&mut self, config: Config) -> Result<(), Error<I2C::Error>> {
        self.write_register(Register::CONFIG, config.bits)?;
        self.config = config;
        Ok(())
    }

    fn write_register(&mut self, register: u8, value: u16) -> Result<(), Error<I2C::Error>> {
        let data = [register, value as u8, (value >> 8) as u8];
        log::debug!(
            "Writing to VEML3328 register 0x{:02X}: 0x{:04X} (bytes: [{}, {}, {}])",
            register,
            value,
            data[0],
            data[1],
            data[2]
        );

        self.i2c.write(DEVICE_ADDRESS, &data).map_err(Error::I2C)?;

        // Add longer delay after write operation for bit-banged I2C
        std::thread::sleep(std::time::Duration::from_millis(10));
        Ok(())
    }

    //add debug function to read all 16 registers
    pub fn read_all_registers(&mut self) -> Result<[u16; 16], Error<I2C::Error>> {
        let mut registers = [0u16; 16];

        log::info!("=== VEML3328 Register Dump (Bit-banged I2C) ===");

        // First verify device communication by reading ID
        match self.read_register(Register::ID_DATA) {
            Ok(id) => log::info!("Device ID verification: 0x{id:04X}"),
            Err(e) => log::warn!("Failed to read device ID: {e:?}"),
        }

        // Read registers 0x00 to 0x0F (0-15)
        for i in 0..16 {
            // Add longer delay between register reads for bit-banged I2C
            std::thread::sleep(std::time::Duration::from_millis(5));

            match self.read_register(i) {
                Ok(value) => {
                    registers[i as usize] = value;
                    let register_name = match i {
                        0x00 => "CONFIG",
                        0x04 => "C_DATA (Clear)",
                        0x05 => "R_DATA (Red)",
                        0x06 => "G_DATA (Green)",
                        0x07 => "B_DATA (Blue)",
                        0x08 => "IR_DATA (Infrared)",
                        0x0C => "ID_DATA (Device ID)",
                        _ => "RESERVED/UNKNOWN",
                    };
                    log::info!("Register 0x{i:02X} ({register_name}): 0x{value:04X} ({value})");
                }
                Err(e) => {
                    log::warn!("Failed to read register 0x{i:02X}: {e:?}");
                    registers[i as usize] = 0; // Set to 0 on error
                }
            }
        }

        log::info!("=== End Register Dump ===");
        Ok(registers)
    }

    fn read_register(&mut self, register: u8) -> Result<u16, Error<I2C::Error>> {
        let mut data = [0; 2];

        // Add small delay before read for bit-banged I2C stability
        std::thread::sleep(std::time::Duration::from_millis(5));

        log::debug!("Reading VEML3328 register 0x{register:02X}");

        self.i2c
            .write_read(DEVICE_ADDRESS, &[register], &mut data)
            .map_err(Error::I2C)?;

        // According to datasheet: data[0] = LSB (bits 7-0), data[1] = MSB (bits 15-8)
        let result = u16::from(data[0]) | (u16::from(data[1]) << 8);

        log::debug!(
            "VEML3328 register 0x{:02X}: raw=[{}, {}], result=0x{:04X} ({})",
            register,
            data[0],
            data[1],
            result,
            result
        );

        Ok(result)
    }
}
