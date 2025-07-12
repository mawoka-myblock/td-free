use esp_hal::i2c::master::I2c;
use veml7700::Veml7700;



fn init_veml(i2c: I2c<'static, esp_hal::Blocking>) -> Veml7700<I2c<'static, esp_hal::Blocking>> {
    let mut veml: Veml7700<I2c<'static, esp_hal::Blocking>> = Veml7700::new(i2c);
    veml.enable().unwrap();
    veml
}

