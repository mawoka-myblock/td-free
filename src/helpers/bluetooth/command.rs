use esp_idf_svc::bt::BdAddr;
use log::{info, warn};

use crate::helpers::{
    auto_calibrate::auto_calibrate,
    bluetooth::server::{BtServer, process_write},
};

impl BtServer {
    pub fn on_command(&self, addr: BdAddr, data: &[u8], offset: u16, handle: u16) {
        let mut unlocked_is_subscribed = self.is_subscribed.lock().unwrap();
        *unlocked_is_subscribed = false;
        warn!("On command: {data:?}");
        let complete_data: Vec<u8> =
            match process_write(&self.write_buffers, addr, handle, data, offset, b'\n') {
                Some(d) => d,
                None => return,
            };
        info!("Full data received");
        let str_data = String::from_utf8(complete_data).unwrap();
        let data = str_data.replace("\n", "");
        let mut response = match &*data {
            "auto_calibrate" => {
                info!("Now running auto calibrate");
                let d = auto_calibrate(
                    self.run_data.rgb.clone(),
                    self.run_data.saved_rgb_multipliers.clone(),
                    self.run_data.nvs.clone(),
                    self.run_data.lux_buffer.clone(),
                );
                d.into_bytes()
            }
            _ => b"unknown_command".to_vec(),
        };
        response.insert(0, b'S');
        self.notify_ind(&response).unwrap();
        info!("Rersponse: {response:?}");
        let mut unlocked_is_subscribed = self.is_subscribed.lock().unwrap();
        *unlocked_is_subscribed = true;
    }
}
