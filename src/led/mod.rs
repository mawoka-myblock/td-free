use std::sync::{Arc, Mutex};

use embedded_hal::delay::DelayNs;
use esp_idf_svc::hal::delay::FreeRtos;
use smart_leds::RGB8;

use crate::LedType;

pub fn set_led(arced_led: Arc<Mutex<LedType>>, red: u8, green: u8, blue: u8) {
    let pixels = std::iter::repeat(RGB8::new(red, green, blue)).take(1);
    let mut led = arced_led.lock().unwrap();
    led.write_nocopy(pixels).unwrap();
}

pub fn show_veml_not_found_error(arced_led: Arc<Mutex<LedType>>) {
    loop {
        log::error!("VEML7700 communication failed!");
        set_led(arced_led.clone(), 255, 0, 0);
        FreeRtos.delay_ms(500u32);
    }
}
