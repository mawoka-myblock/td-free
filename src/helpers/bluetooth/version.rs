use esp_idf_svc::{
    bt::ble::gatt::{
        GattInterface, GattStatus, Handle,
        server::{ConnectionId, TransferId},
    },
    sys::EspError,
};

use crate::helpers::bluetooth::server::BtServer;

impl BtServer {
    pub fn read_version(
        &self,
        gatt_if: GattInterface,
        conn_id: ConnectionId,
        trans_id: TransferId,
        offset: u16,
        attr_handle: Handle,
    ) -> Result<(), EspError> {
        let color_available = self.run_data.rgb.is_some();
        let version = option_env!("TD_FREE_VERSION").unwrap_or("UNKNOWN");
        let data = format!(
            r#"{{"spoolman_available": false, "color_available": {color_available},"version": "{version}"}}"#,
        );
        let bytes_json = data.as_bytes();
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
