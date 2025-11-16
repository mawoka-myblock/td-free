use esp_idf_svc::bt::BdAddr;

use crate::helpers::bluetooth::server::{BtServer, process_write};

impl BtServer {
    pub fn on_command(&self, addr: BdAddr, data: &[u8], offset: u16, handle: u16) {
        let complete_data: Vec<u8> =
            match process_write(&self.write_buffers, addr, handle, data, offset, b'\n') {
                Some(d) => d,
                None => return,
            };
        self.notify_command(&complete_data).unwrap();
    }
}
