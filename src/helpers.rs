use anyhow::bail;
use esp_idf_svc::{
    nvs::{EspNvs, EspNvsPartition, NvsDefault},
    sys::esp_random,
};
use log::warn;

#[derive(Debug, Clone, Copy)]
pub struct NvsData {
    pub b: f32,
    pub m: f32,
    pub threshold: f32,
}

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
