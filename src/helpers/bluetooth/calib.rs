use esp_idf_svc::{
    bt::{
        BdAddr,
        ble::gatt::{
            GattInterface, GattStatus, Handle,
            server::{ConnectionId, TransferId},
        },
    },
    sys::EspError,
};
use log::{error, info, warn};

use crate::helpers::{
    bluetooth::server::{BtServer, process_write},
    nvs::{RGBMultipliers, save_rgb_multipliers},
};

impl BtServer {
    pub fn on_calib(&self, addr: BdAddr, data: &[u8], offset: u16, handle: u16) {
        warn!("On calib called");
        let complete_data: Vec<u8> =
            match process_write(&self.write_buffers, addr, handle, data, offset, b'}') {
                Some(d) => d,
                None => return,
            };

        let str_data = String::from_utf8(complete_data).unwrap();
        let mut red = 1.0f32;
        let mut green = 1.0f32;
        let mut blue = 1.0f32;
        let mut brightness = 1.0f32;
        let mut reference_r = 127u8;
        let mut reference_g = 127u8;
        let mut reference_b = 127u8;

        for part in str_data.split(',') {
            let part = part.trim().trim_matches('{').trim_matches('}');
            if let Some((key, value)) = part.split_once(':') {
                let key = key.trim().trim_matches('"');
                let value = value.trim();

                match key {
                    "red" => red = value.parse().unwrap_or(1.0),
                    "green" => green = value.parse().unwrap_or(1.0),
                    "blue" => blue = value.parse().unwrap_or(1.0),
                    "brightness" => brightness = value.parse().unwrap_or(1.0),
                    "reference_r" => reference_r = value.parse().unwrap_or(127),
                    "reference_g" => reference_g = value.parse().unwrap_or(127),
                    "reference_b" => reference_b = value.parse().unwrap_or(127),
                    _ => {}
                }
            }
        }

        // Clamp values to reasonable ranges
        red = red.clamp(0.1, 5.0);
        green = green.clamp(0.1, 5.0);
        blue = blue.clamp(0.1, 5.0);
        brightness = brightness.clamp(0.1, 5.0);

        // Get current TD reference to preserve it
        let current_td_reference = {
            let multipliers = self.run_data.saved_rgb_multipliers.lock().unwrap();
            multipliers.td_reference
        };

        let new_multipliers = RGBMultipliers {
            red,
            green,
            blue,
            brightness,
            td_reference: current_td_reference,
            reference_r,
            reference_g,
            reference_b,
        };
        {
            let mut multipliers = self.run_data.saved_rgb_multipliers.lock().unwrap();
            *multipliers = new_multipliers;
        }
        match save_rgb_multipliers(new_multipliers, self.run_data.nvs.as_ref().clone()) {
            Ok(_) => info!("RGB Multipliers saved successfully"),
            Err(e) => error!("Couldn't save RGB multipliers: {e:?}"),
        }
    }

    pub fn read_calib(
        &self,
        gatt_if: GattInterface,
        conn_id: ConnectionId,
        trans_id: TransferId,
        offset: u16,
        attr_handle: Handle,
    ) -> Result<(), EspError> {
        info!("Read RGB Multipliers");
        let multipliers = {
            let d = self.run_data.saved_rgb_multipliers.lock().unwrap();
            *d
        };
        let json_response = format!(
            r#"{{"red": {:.2}, "green": {:.2}, "blue": {:.2}, "brightness": {:.2}, "td_reference": {:.2}, "reference_r": {}, "reference_g": {}, "reference_b": {}, "rgb_disabled": true}}"#,
            multipliers.red,
            multipliers.green,
            multipliers.blue,
            multipliers.brightness,
            multipliers.td_reference,
            multipliers.reference_r,
            multipliers.reference_g,
            multipliers.reference_b
        );
        let bytes_json = json_response.as_bytes();
        let chunk = &bytes_json[offset as usize..];
        let mut state = self.state.lock().unwrap();
        state
            .response
            .attr_handle(attr_handle)
            .value(chunk)
            .unwrap();
        self.gatts.send_response(
            gatt_if,
            conn_id,
            trans_id,
            GattStatus::Ok,
            Some(&state.response),
        )?;
        Ok(())
    }
}
