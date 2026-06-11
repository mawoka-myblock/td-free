use defmt::{Debug2Format, info, unwrap};
use embassy_time::Timer;
use embedded_hal_async::i2c::I2c as _;
use esp_hal::{
    Blocking,
    gpio::Flex,
    i2c::master::{Config, I2c},
    peripherals::{self, I2C0},
    time::Rate,
};
use veml7700::Veml7700;

use crate::helpers::{bitbang_i2c, veml3328::VEML3328};

#[embassy_executor::task]
pub async fn sensor_task(
    _sda_v77: peripherals::GPIO6<'static>,
    _scl_v77: peripherals::GPIO5<'static>,
    _sda_v33: peripherals::GPIO8<'static>,
    _scl_v33: peripherals::GPIO10<'static>,
    _i2c_per: peripherals::I2C0<'static>,
) {
    loop {
        {
            let mut v77 = get_v77();
            v77.enable().unwrap();
            info!("v77: {}", v77.read_lux().unwrap());
        }

        {
            let mut v33 = get_v33();
            v33.enable().ok();
            info!("v33: {}", v33.read_clear().unwrap());
        }

        Timer::after_millis(1000).await;
    }
}

fn get_i2c<'d>() -> I2c<'d, Blocking> {
    I2c::new(
        unsafe { peripherals::I2C0::steal() },
        Config::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
}

fn get_v33<'d>() -> VEML3328<I2c<'d, Blocking>> {
    let i2c = get_i2c().with_sda(v33_sda()).with_scl(v33_scl());
    VEML3328::new(i2c)
}

fn get_v77<'d>() -> Veml7700<I2c<'d, Blocking>> {
    let i2c = get_i2c().with_sda(v77_sda()).with_scl(v77_scl());
    Veml7700::new(i2c)
}

fn v77_sda() -> peripherals::GPIO6<'static> {
    unsafe { peripherals::GPIO6::steal() }
}

fn v77_scl() -> peripherals::GPIO5<'static> {
    unsafe { peripherals::GPIO5::steal() }
}

fn v33_sda() -> peripherals::GPIO8<'static> {
    unsafe { peripherals::GPIO8::steal() }
}

fn v33_scl() -> peripherals::GPIO10<'static> {
    unsafe { peripherals::GPIO10::steal() }
}
