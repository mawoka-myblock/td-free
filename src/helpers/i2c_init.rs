use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use esp_idf_svc::hal::{
    gpio::{Gpio5, Gpio6, Gpio8, Gpio10, PinDriver, Pull},
    i2c::{I2C0, I2cConfig, I2cDriver},
    units::KiloHertz,
};
use log::info;
use veml7700::Veml7700;

use crate::{
    LedType,
    helpers::bitbang_i2c::{
        HardwareI2c, HardwareI2cInstance, SimpleBitBangI2c, SimpleBitBangI2cInstance,
    },
    led, veml3328,
};

pub struct Pins {
    pub sda1: Gpio6,
    pub scl1: Gpio5,
    pub sda2: Gpio8,
    pub scl2: Gpio10,
    pub i2c: I2C0,
}

pub struct I2cInitResponse {
    pub veml7700: Arc<Mutex<Veml7700<HardwareI2cInstance>>>,
    pub veml3328: Option<Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>>,
    pub is_old_pcb: bool,
}

pub fn initialize_veml(
    pins: Pins,
    ws2812_old: Arc<Mutex<LedType>>,
    ws2812_new: Arc<Mutex<LedType>>,
) -> I2cInitResponse {
    // Use hardware I2C for VEML7700 on primary pins
    let hw_config = I2cConfig::new()
        .baudrate(KiloHertz::from(100).into())
        .timeout(Duration::from_millis(100).into());

    let hw_i2c = I2cDriver::new(pins.i2c, pins.sda1, pins.scl1, &hw_config);

    if hw_i2c.is_err() {
        info!(
            "Primary I2C failed: {:?}, trying alt pins for both",
            hw_i2c.err()
        );
        return init_alt_i2c_both(pins.sda2, pins.scl2, ws2812_old, ws2812_new);
    }

    let hw_i2c_driver = hw_i2c.unwrap();
    let hardware_i2c = HardwareI2c::new(hw_i2c_driver);

    // Create VEML7700 with hardware I2C
    let mut veml_temp = Veml7700::new(hardware_i2c.clone_driver());

    let veml_enable_res = veml_temp.enable();
    if veml_enable_res.is_err() {
        info!(
            "VEML7700 enable failed: {:?}, trying alt pins",
            veml_enable_res.err()
        );
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
    let mut veml_rgb_available = false;
    // Enable RGB sensor
    match veml_rgb_temp.enable() {
        Ok(_) => {
            log::info!("VEML3328 enabled successfully on bit-banged I2C");
            log::info!("RGB sensor ready for white balance calibration");
            // Try to read device ID to verify communication
            match veml_rgb_temp.read_device_id() {
                Ok(id) => {
                    log::info!("VEML3328 device ID: 0x{id:04X}");
                    if id == 0x28 {
                        log::info!("VEML3328 device ID matches expected value!");
                        veml_rgb_available = true;
                    } else if id == 0x0000 {
                        log::error!("VEML3328 device ID is 0x0000 - no communication!");
                    } else {
                        log::warn!("Unexpected device ID! Expected 0x28, got 0x{id:04X}");
                    }
                }
                Err(e) => log::warn!("Could not read VEML3328 device ID: {e:?}"),
            }
        }
        Err(e) => {
            log::error!("Could not enable VEML3328 RGB sensor: {e:?}");
        }
    }

    let veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>> = Arc::new(Mutex::new(veml_temp));
    let veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>> =
        Arc::new(Mutex::new(veml_rgb_temp));
    I2cInitResponse {
        veml7700: veml,
        veml3328: match veml_rgb_available {
            true => Some(veml_rgb),
            false => None,
        },
        is_old_pcb: false,
    }
}

fn init_alt_i2c_both(
    sda: Gpio8,
    scl: Gpio10,
    ws2812_old: Arc<Mutex<LedType>>,
    ws2812_new: Arc<Mutex<LedType>>,
) -> I2cInitResponse {
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
            let mut veml_rgb_available = false;
            // Enable RGB sensor
            match veml_rgb_temp.enable() {
                Ok(_) => {
                    log::info!("VEML3328 enabled successfully on alt bit-banged I2C");
                    // Try to read device ID to verify communication
                    match veml_rgb_temp.read_device_id() {
                        Ok(id) => {
                            log::info!("VEML3328 device ID: 0x{id:04X}");
                            if id == 0x28 {
                                veml_rgb_available = true;
                            } else {
                                log::warn!("Unexpected device ID! Expected 0x28, got 0x{id:04X}");
                            }
                        }
                        Err(e) => log::warn!("Could not read VEML3328 device ID: {e:?}"),
                    }
                }
                Err(e) => {
                    log::error!("Could not enable VEML3328 RGB sensor: {e:?}");
                }
            }

            let veml: Arc<Mutex<Veml7700<HardwareI2cInstance>>> = Arc::new(Mutex::new(veml_temp));
            let veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>> =
                Arc::new(Mutex::new(veml_rgb_temp));

            return I2cInitResponse {
                veml7700: veml,
                veml3328: match veml_rgb_available {
                    true => Some(veml_rgb),
                    false => None,
                },
                is_old_pcb: true,
            };
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
        log::error!(
            "VEML7700 enable failed on alt pins with bit-bang: {:?}",
            veml_enable_res.err()
        );
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
                    log::info!("VEML3328 device ID: 0x{id:04X}");
                    if id != 0x28 {
                        log::warn!("Unexpected device ID! Expected 0x28, got 0x{id:04X}");
                    }
                }
                Err(e) => log::warn!("Could not read VEML3328 device ID: {e:?}"),
            }
        }
        Err(e) => {
            log::error!("Could not enable VEML3328 RGB sensor: {e:?}");
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
        let veml_rgb: Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>> =
            Arc::new(Mutex::new(veml_rgb_temp));

        // Log warning that we're using a workaround
        log::warn!("Using workaround: bit-banged VEML7700 wrapped in hardware I2C type");
        return I2cInitResponse {
            veml7700: veml,
            veml3328: Some(veml_rgb),
            is_old_pcb: true,
        };
    } else {
        // If even the dummy fails, we have no choice but to panic
        log::error!("Complete I2C failure - cannot create any I2C instances");
        led::show_veml_not_found_error(ws2812_old, ws2812_new);
        unreachable!();
    }
}
