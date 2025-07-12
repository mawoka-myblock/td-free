use std::sync::{Arc, Mutex};

use embedded_hal::delay::DelayNs;
use esp_idf_svc::hal::delay::FreeRtos;
use smart_leds::RGB8;

use crate::LedType;

pub fn set_led(arced_led: Arc<Mutex<LedType>>, red: u8, green: u8, blue: u8) {
    let pixels = std::iter::repeat_n(RGB8::new(red, green, blue),1);
    let mut led = arced_led.lock().unwrap();
    led.write_nocopy(pixels).unwrap();
}

pub fn show_veml_not_found_error(arced_led_old: Arc<Mutex<LedType>>, arced_led_new: Arc<Mutex<LedType>>) {
    loop {
        log::error!("VEML7700 communication failed!");
        set_led(arced_led_old.clone(), 255, 0, 0);
        set_led(arced_led_new.clone(), 255, 0, 0);
        FreeRtos.delay_ms(500u32);
    }
}
