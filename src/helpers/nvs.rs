use anyhow::bail;
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::{error, info, warn};

#[derive(Debug, Clone, Copy)]
pub struct NvsData {
    pub b: f32,
    pub m: f32,
    pub threshold: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct RGBMultipliers {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub brightness: f32,
    pub td_reference: f32, // TD value at calibration time
    pub reference_r: u8,   // Reference red value (0-255)
    pub reference_g: u8,   // Reference green value (0-255)
    pub reference_b: u8,   // Reference blue value (0-255)
}

impl Default for RGBMultipliers {
    fn default() -> Self {
        Self {
            red: 1.0,
            green: 1.0,
            blue: 1.0,
            brightness: 1.0,
            td_reference: 50.0, // Default to 50% transmission
            reference_r: 127,   // Default to 50% grey
            reference_g: 127,   // Default to 50% grey
            reference_b: 127,   // Default to 50% grey
        }
    }
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

pub fn get_saved_rgb_multipliers(nvs: EspNvsPartition<NvsDefault>) -> RGBMultipliers {
    let nvs = match EspNvs::new(nvs, "rgb_mult", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            warn!("RGB multipliers NVS init failed");
            return RGBMultipliers::default();
        }
    };

    // Use smaller buffers to save memory
    let mut red_buffer = [0u8; 32];
    let red_value: f32 = nvs
        .get_str("red", &mut red_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);

    let mut green_buffer = [0u8; 32];
    let green_value: f32 = nvs
        .get_str("green", &mut green_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);

    let mut blue_buffer = [0u8; 32];
    let blue_value: f32 = nvs
        .get_str("blue", &mut blue_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);

    let mut brightness_buffer = [0u8; 32];
    let brightness_value: f32 = nvs
        .get_str("brightness", &mut brightness_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);

    let mut td_ref_buffer = [0u8; 32];
    let td_reference: f32 = nvs
        .get_str("td_reference", &mut td_ref_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(50.0);

    let mut ref_r_buffer = [0u8; 32];
    let reference_r: u8 = nvs
        .get_str("ref_r", &mut ref_r_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(127);

    let mut ref_g_buffer = [0u8; 32];
    let reference_g: u8 = nvs
        .get_str("ref_g", &mut ref_g_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(127);

    let mut ref_b_buffer = [0u8; 32];
    let reference_b: u8 = nvs
        .get_str("ref_b", &mut ref_b_buffer)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(127);

    RGBMultipliers {
        red: red_value,
        green: green_value,
        blue: blue_value,
        brightness: brightness_value,
        td_reference,
        reference_r,
        reference_g,
        reference_b,
    }
}

pub fn save_rgb_multipliers(
    multipliers: RGBMultipliers,
    nvs: EspNvsPartition<NvsDefault>,
) -> anyhow::Result<()> {
    let mut nvs = match EspNvs::new(nvs, "rgb_mult", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            bail!("RGB multipliers NVS failed");
        }
    };

    nvs.set_str("red", &multipliers.red.to_string())?;
    nvs.set_str("green", &multipliers.green.to_string())?;
    nvs.set_str("blue", &multipliers.blue.to_string())?;
    nvs.set_str("brightness", &multipliers.brightness.to_string())?;
    nvs.set_str("td_reference", &multipliers.td_reference.to_string())?;
    nvs.set_str("ref_r", &multipliers.reference_r.to_string())?;
    nvs.set_str("ref_g", &multipliers.reference_g.to_string())?;
    nvs.set_str("ref_b", &multipliers.reference_b.to_string())?;

    log::info!(
        "Saved RGB multipliers: R={:.2}, G={:.2}, B={:.2}, Brightness={:.2}, TD_ref={:.2}, Ref_RGB=({},{},{})",
        multipliers.red,
        multipliers.green,
        multipliers.blue,
        multipliers.brightness,
        multipliers.td_reference,
        multipliers.reference_r,
        multipliers.reference_g,
        multipliers.reference_b
    );
    Ok(())
}

// Add function to clear corrupted NVS data if needed
pub fn clear_rgb_multipliers_nvs(nvs: EspNvsPartition<NvsDefault>) -> anyhow::Result<()> {
    match EspNvs::new(nvs, "rgb_mult", true) {
        Ok(mut nvs_handle) => {
            warn!("Clearing potentially corrupted RGB multipliers NVS data");
            let _ = nvs_handle.remove("red");
            let _ = nvs_handle.remove("green");
            let _ = nvs_handle.remove("blue");
            let _ = nvs_handle.remove("brightness");
            let _ = nvs_handle.remove("td_reference");
            let _ = nvs_handle.remove("ref_r");
            let _ = nvs_handle.remove("ref_g");
            let _ = nvs_handle.remove("ref_b");
            info!("RGB multipliers NVS data cleared");
            Ok(())
        }
        Err(e) => {
            bail!("Failed to open RGB multipliers NVS for clearing: {e:?}");
        }
    }
}

pub fn save_spoolman_data(
    url: &str,
    field_name: &str,
    nvs: EspNvsPartition<NvsDefault>,
) -> anyhow::Result<()> {
    let mut nvs = match EspNvs::new(nvs, "prefs", true) {
        Ok(nvs) => nvs,
        Err(_) => {
            bail!("NVS failed");
        }
    };
    info!("Saving Spoolman: {url}");
    nvs.set_str("spool_url", url)?; // Changed from "spoolman_url" (11 chars) to "spool_url" (9 chars)
    nvs.set_str("spool_field", field_name)?; // Changed from "spoolman_field_name" (18 chars) to "spool_field" (11 chars)
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
        .get_str("spool_url", &mut spoolman_url_buf) // Changed from "spoolman_url"
        .unwrap_or(None)
        .map(|s| s.to_string());
    let mut spoolman_field_name_buf = vec![0; 256];
    let field_name = nvs
        .get_str("spool_field", &mut spoolman_field_name_buf) // Changed from "spoolman_field_name"
        .unwrap_or(None)
        .map(|s| s.to_string());
    (url, field_name)
}
