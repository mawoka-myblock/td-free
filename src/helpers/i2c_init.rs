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
    // Only new TD
    pub sda1: Gpio6,
    pub scl1: Gpio5,
    // Old TD, New RGB
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
    let hw_i2c = I2cDriver::new(pins.i2c, pins.sda1, pins.scl1, &create_i2c_config());
    let mut veml7700 = Veml7700::new(HardwareI2c::new(hw_i2c.unwrap()).clone_driver());
    if veml7700.enable().is_err() {
        info!("Assuming old");
        // return HW i2c on sda2 and scl2, is old and no rgb
        drop(veml7700);
        let hw_i2c_alt = match I2cDriver::new(
            unsafe { esp_idf_svc::hal::i2c::I2C0::new() },
            unsafe { Gpio8::new() },
            unsafe { Gpio10::new() },
            &create_i2c_config(),
        ) {
            Ok(d) => d,
            Err(_) => {
                led::show_veml_not_found_error(ws2812_old, ws2812_new);
                unreachable!()
            }
        };
        let mut veml = Veml7700::new(HardwareI2c::new(hw_i2c_alt).clone_driver());
        if veml.enable().is_err() {
            led::show_veml_not_found_error(ws2812_old, ws2812_new);
            unreachable!()
        }
        return I2cInitResponse {
            is_old_pcb: true,
            veml3328: None,
            veml7700: Arc::new(Mutex::new(veml)),
        };
    }
    let rgb_veml = get_rgb_veml(pins.sda2, pins.scl2);
    I2cInitResponse {
        veml7700: Arc::new(Mutex::new(veml7700)),
        veml3328: rgb_veml,
        is_old_pcb: false,
    }
    // Check if sda2 and scl2 are connected for rgb sensor
}

fn get_rgb_veml(
    sda: Gpio8,
    scl: Gpio10,
) -> Option<Arc<Mutex<veml3328::VEML3328<SimpleBitBangI2cInstance>>>> {
    let mut sda_pin = PinDriver::input_output(sda).unwrap();
    let mut scl_pin = PinDriver::input_output(scl).unwrap();

    // Initialize pins to idle state (both high with pull-up)
    sda_pin.set_pull(Pull::Up).unwrap();
    scl_pin.set_pull(Pull::Up).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let bitbang_i2c = SimpleBitBangI2c::new(sda_pin, scl_pin);
    let mut veml_rgb_temp = veml3328::VEML3328::new(bitbang_i2c.clone_driver());
    if veml_rgb_temp.enable().is_err() {
        info!("Enable error!");
        return None;
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    if let Ok(veml_id) = veml_rgb_temp.read_device_id() {
        info!("id Check");
    /*        if veml_id != 0x28 {
        info!("Wrong ID, received 0x{veml_id:04X}");
        return None;
    } */
    } else {
        info!("ID read error");
        return None;
    }
    Some(Arc::new(Mutex::new(veml_rgb_temp)))
}

fn create_i2c_config() -> I2cConfig {
    I2cConfig::new()
        .baudrate(KiloHertz::from(100).into())
        .timeout(Duration::from_millis(100).into())
}
