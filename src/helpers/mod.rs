pub mod baseline_readings;
pub mod bitbang_i2c;
pub mod i2c_init;
pub mod median_buffer;
pub mod nvs;
pub mod readings;
pub mod rgb;
pub mod serial;

use esp_idf_svc::sys::esp_random;

// Simplified bit-bang I2C implementation using a different approach

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
