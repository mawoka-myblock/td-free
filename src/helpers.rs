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
        gpio::{Gpio10, Gpio5, Gpio6, Gpio8},
        i2c::{I2cConfig, I2cDriver, I2C0},
        peripheral::Peripheral,
    },
    nvs::{EspNvs, EspNvsPartition, NvsDefault},
    sys::esp_random,
};
use veml7700::Veml7700;

use crate::{led, LedType};
use esp_idf_svc::hal::prelude::*;

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
    mut pins: Pins,
    ws2812_old: Arc<Mutex<LedType>>,
    ws2812_new: Arc<Mutex<LedType>>,
) -> (Arc<Mutex<Veml7700<I2cDriver<'static>>>>, bool) {
    let config = I2cConfig::new()
        .baudrate(KiloHertz::from(20).into())
        .timeout(Duration::from_millis(100).into());

    // Try GPIO6 and 5
    let i2c_0 = I2cDriver::new(
        unsafe { pins.i2c.clone_unchecked() },
        pins.sda1,
        pins.scl1,
        &config,
    );
    if i2c_0.is_err() {
        info!("Trying alt i2c before veml enable");
        drop(i2c_0.unwrap());
        return (
            init_alt_i2c(pins.sda2, pins.scl2, pins.i2c, ws2812_old, ws2812_new),
            true,
        );
    }
    let mut veml_temp = Veml7700::new(i2c_0.unwrap());

    let veml_enable_res = veml_temp.enable();
    if veml_enable_res.is_err() {
        drop(veml_temp.destroy());
        info!("Trying alt i2c after veml enable");
        return (
            init_alt_i2c(pins.sda2, pins.scl2, pins.i2c, ws2812_old, ws2812_new),
            true,
        );
    }

    let veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>> = Arc::new(Mutex::new(veml_temp));
    (veml, false)
}

fn init_alt_i2c(
    sda: Gpio8,
    scl: Gpio10,
    i2c: I2C0,
    ws2812_old: Arc<Mutex<LedType>>,
    ws2812_new: Arc<Mutex<LedType>>,
) -> Arc<Mutex<Veml7700<I2cDriver<'static>>>> {
    let config = I2cConfig::new()
        .baudrate(KiloHertz::from(20).into())
        .timeout(Duration::from_millis(100).into());
    let i2c_0 = I2cDriver::new(i2c, sda, scl, &config);
    if i2c_0.is_err() {
        led::show_veml_not_found_error(ws2812_old, ws2812_new);
        unreachable!();
    }
    let mut veml_temp = Veml7700::new(i2c_0.unwrap());

    let veml_enable_res = veml_temp.enable();
    if veml_enable_res.is_err() {
        led::show_veml_not_found_error(ws2812_old, ws2812_new);
        unreachable!();
    }

    let veml: Arc<Mutex<Veml7700<I2cDriver<'_>>>> = Arc::new(Mutex::new(veml_temp));
    veml
}
