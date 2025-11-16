use enumset::enum_set;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::RgbWsHandler;
use crate::helpers::bluetooth::{
    APP_ID, CALIB_CHARACTERISTIC_UUID, COMMAND_WRITE, ExEspBleGap, ExEspGatts,
    IND_CHARACTERISTIC_UUID, MAX_CONNECTIONS, SERVICE_UUID,
};
use crate::helpers::median_buffer;
use crate::helpers::nvs::RGBMultipliers;
use esp_idf_svc::sys::{ESP_FAIL, EspError};

use esp_idf_svc::bt::ble::gap::{AdvConfiguration, BleGapEvent};
use esp_idf_svc::bt::ble::gatt::server::{ConnectionId, GattsEvent, TransferId};
use esp_idf_svc::bt::ble::gatt::{
    AutoResponse, GattCharacteristic, GattDescriptor, GattId, GattInterface, GattResponse,
    GattServiceId, GattStatus, Handle, Permission, Property,
};
use esp_idf_svc::bt::{BdAddr, BtStatus, BtUuid};
use log::{info, warn};

#[derive(Debug, Clone)]
pub struct Connection {
    pub peer: BdAddr,
    pub conn_id: Handle,
    pub subscribed: bool,
    pub mtu: Option<u16>,
}

#[derive(Default)]
pub struct State {
    pub gatt_if: Option<GattInterface>,
    pub service_handle: Option<Handle>,
    pub recv_handle: Option<Handle>,
    pub ind_handle: Option<Handle>,
    pub ind_cccd_handle: Option<Handle>,
    pub calib_handle: Option<Handle>,
    pub command_handle: Option<Handle>,
    pub connections: heapless::Vec<Connection, MAX_CONNECTIONS>,
    pub response: GattResponse,
}

type WriteBufferMap = HashMap<([u8; 6], Handle), Vec<u8>>;

#[derive(Clone)]
pub struct BtServer {
    pub gap: ExEspBleGap,
    pub gatts: ExEspGatts,
    pub state: Arc<Mutex<State>>,
    pub run_data: RunData,
    pub is_subscribed: Arc<Mutex<bool>>,
    pub write_buffers: RefCell<WriteBufferMap>,
}

#[derive(Clone)]
pub struct RunData {
    pub nvs: Arc<EspNvsPartition<NvsDefault>>,
    // Add median buffers
    pub lux_buffer: Arc<Mutex<median_buffer::RunningMedianBuffer>>,
    pub rgb: Option<RgbWsHandler>,
    pub saved_rgb_multipliers: Arc<Mutex<RGBMultipliers>>,
}

impl BtServer {
    pub fn new(gap: ExEspBleGap, gatts: ExEspGatts, data: RunData) -> Self {
        Self {
            gap,
            gatts,
            state: Arc::new(Mutex::new(Default::default())),
            run_data: data,
            is_subscribed: Arc::new(Mutex::new(false)),
            write_buffers: RefCell::new(HashMap::new()),
        }
    }
    fn on_subscribed(&self, _: BdAddr) {
        let mut unlocked_is_subscribed = self.is_subscribed.lock().unwrap();
        *unlocked_is_subscribed = true;
    }
    fn on_unsubscribed(&self, _: BdAddr) {
        let mut unlocked_is_subscribed = self.is_subscribed.lock().unwrap();
        *unlocked_is_subscribed = false;
    }
    fn on_recv(&self, _: BdAddr, _: &[u8], _: u16, _: Option<u16>) {}

    pub fn on_gap_event(&self, event: BleGapEvent) -> Result<(), EspError> {
        info!("Got event: {event:?}");

        if let BleGapEvent::AdvertisingConfigured(status) = event {
            self.check_bt_status(status)?;
            self.gap.start_advertising()?;
        }

        Ok(())
    }
    /*
       pub fn indicate(&self, data: &[u8]) -> Result<(), EspError> {
           for peer_index in 0..MAX_CONNECTIONS {
               let mut state = self.state.lock().unwrap();

               loop {
                   if state.connections.len() <= peer_index {
                       break;
                   }

                   let Some(gatt_if) = state.gatt_if else {
                       break;
                   };
                   let Some(ind_handle) = state.ind_handle else {
                       break;
                   };

                   if state.ind_confirmed.is_none() {
                       let conn = &state.connections[peer_index];
                       self.gatts
                           .indicate(gatt_if, conn.conn_id, ind_handle, data)?;
                       state.ind_confirmed = Some(conn.peer);
                       break;
                   } else {
                       state = self.condvar.wait(state).unwrap();
                   }
               }
           }
           Ok(())
       }
    */
    pub fn notify_ind(&self, data: &[u8]) -> Result<(), EspError> {
        let state = self.state.lock().unwrap();

        if let Some(gatt_if) = state.gatt_if {
            if let Some(notify_handle) = state.ind_handle {
                for conn in state.connections.iter() {
                    if conn.subscribed {
                        self.gatts
                            .notify(gatt_if, conn.conn_id, notify_handle, data)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn on_gatts_event(
        &self,
        gatt_if: GattInterface,
        event: GattsEvent,
    ) -> Result<(), EspError> {
        info!("Got event: {event:?}");

        match event {
            GattsEvent::ServiceRegistered { status, app_id } => {
                self.check_gatt_status(status)?;
                if APP_ID == app_id {
                    self.create_service(gatt_if)?;
                }
            }
            GattsEvent::ServiceCreated {
                status,
                service_handle,
                ..
            } => {
                self.check_gatt_status(status)?;
                self.configure_and_start_service(service_handle)?;
            }
            GattsEvent::CharacteristicAdded {
                status,
                attr_handle,
                service_handle,
                char_uuid,
            } => {
                self.check_gatt_status(status)?;
                self.register_characteristic(service_handle, attr_handle, char_uuid)?;
            }
            GattsEvent::DescriptorAdded {
                status,
                attr_handle,
                service_handle,
                descr_uuid,
            } => {
                self.check_gatt_status(status)?;
                self.register_cccd_descriptor(service_handle, attr_handle, descr_uuid)?;
            }
            GattsEvent::ServiceDeleted {
                status,
                service_handle,
            } => {
                self.check_gatt_status(status)?;
                self.delete_service(service_handle)?;
            }
            GattsEvent::ServiceUnregistered {
                status,
                service_handle,
                ..
            } => {
                self.check_gatt_status(status)?;
                self.unregister_service(service_handle)?;
            }
            GattsEvent::Mtu { conn_id, mtu } => {
                self.register_conn_mtu(conn_id, mtu)?;
            }
            GattsEvent::PeerConnected { conn_id, addr, .. } => {
                self.create_conn(conn_id, addr)?;
            }
            GattsEvent::PeerDisconnected { addr, .. } => {
                self.delete_conn(addr)?;
            }
            GattsEvent::Write {
                conn_id,
                trans_id,
                addr,
                handle,
                offset,
                need_rsp,
                is_prep,
                value,
            } => {
                let handled = self.recv(
                    gatt_if, conn_id, trans_id, addr, handle, offset, need_rsp, is_prep, value,
                )?;

                if handled {
                    self.send_write_response(
                        gatt_if, conn_id, trans_id, handle, offset, need_rsp, is_prep, value,
                    )?;
                }
            }
            GattsEvent::Confirm { status, .. } => {
                self.check_gatt_status(status)?;
                // self.confirm_indication()?;
            }
            GattsEvent::ExecWrite {
                conn_id,
                trans_id,
                addr: _,
                canceled: _,
            } => {
                self.gatts
                    .send_response(gatt_if, conn_id, trans_id, GattStatus::Ok, None)?;
            }
            GattsEvent::Read {
                conn_id,
                trans_id,
                addr: _,
                handle,
                offset,
                is_long: _,
                need_rsp,
            } => {
                let should_read = {
                    let state = self.state.lock().unwrap();
                    Some(handle) == state.calib_handle && need_rsp
                };

                if should_read {
                    self.read_calib(gatt_if, conn_id, trans_id, offset, handle)?;
                }
            }
            _ => (),
        }

        Ok(())
    }

    /// Create the service and start advertising
    /// Called from within the event callback once we are notified that the GATTS app is registered
    fn create_service(&self, gatt_if: GattInterface) -> Result<(), EspError> {
        self.state.lock().unwrap().gatt_if = Some(gatt_if);

        self.gap.set_device_name("Td-Free")?;
        self.gap.set_adv_conf(&AdvConfiguration {
            include_name: true,
            include_txpower: true,
            flag: 2,
            service_uuid: Some(BtUuid::uuid128(SERVICE_UUID)),
            // service_data: todo!(),
            // manufacturer_data: todo!(),
            ..Default::default()
        })?;
        self.gatts.create_service(
            gatt_if,
            &GattServiceId {
                id: GattId {
                    uuid: BtUuid::uuid128(SERVICE_UUID),
                    inst_id: 0,
                },
                is_primary: true,
            },
            12,
        )?;

        Ok(())
    }

    /// Delete the service
    /// Called from within the event callback once we are notified that the GATTS app is deleted
    fn delete_service(&self, service_handle: Handle) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if state.service_handle == Some(service_handle) {
            state.recv_handle = None;
            state.ind_handle = None;
            state.ind_cccd_handle = None;
        }

        Ok(())
    }

    /// Unregister the service
    /// Called from within the event callback once we are notified that the GATTS app is unregistered
    fn unregister_service(&self, service_handle: Handle) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if state.service_handle == Some(service_handle) {
            state.gatt_if = None;
            state.service_handle = None;
        }

        Ok(())
    }

    /// Configure and start the service
    /// Called from within the event callback once we are notified that the service is created
    fn configure_and_start_service(&self, service_handle: Handle) -> Result<(), EspError> {
        self.state.lock().unwrap().service_handle = Some(service_handle);

        self.gatts.start_service(service_handle)?;
        self.add_characteristics(service_handle)?;

        Ok(())
    }

    /// Add our two characteristics to the service
    /// Called from within the event callback once we are notified that the service is created
    fn add_characteristics(&self, service_handle: Handle) -> Result<(), EspError> {
        self.gatts.add_characteristic(
            service_handle,
            &GattCharacteristic {
                uuid: BtUuid::uuid128(CALIB_CHARACTERISTIC_UUID),
                permissions: enum_set!(Permission::Read | Permission::Write),
                properties: enum_set!(Property::Read | Property::Write),
                max_len: 512,
                auto_rsp: AutoResponse::ByApp,
            },
            &[],
        )?;

        self.gatts.add_characteristic(
            service_handle,
            &GattCharacteristic {
                uuid: BtUuid::uuid128(COMMAND_WRITE),
                permissions: enum_set!(Permission::Write),
                properties: enum_set!(Property::Write),
                max_len: 512,
                auto_rsp: AutoResponse::ByApp,
            },
            &[],
        )?;

        self.gatts.add_characteristic(
            service_handle,
            &GattCharacteristic {
                uuid: BtUuid::uuid128(IND_CHARACTERISTIC_UUID),
                permissions: enum_set!(Permission::Write | Permission::Read),
                properties: enum_set!(Property::Notify),
                max_len: 200, // Mac iondicate data
                auto_rsp: AutoResponse::ByApp,
            },
            &[],
        )?;

        Ok(())
    }

    /// Add the CCCD descriptor
    /// Called from within the event callback once we are notified that a char descriptor is added,
    /// however the method will do something only if the added char is the "indicate" characteristics of course
    fn register_characteristic(
        &self,
        service_handle: Handle,
        attr_handle: Handle,
        char_uuid: BtUuid,
    ) -> Result<(), EspError> {
        let indicate_char = {
            let mut state = self.state.lock().unwrap();

            if state.service_handle != Some(service_handle) {
                false
            } else if char_uuid == BtUuid::uuid128(CALIB_CHARACTERISTIC_UUID) {
                state.calib_handle = Some(attr_handle);
                false
            } else if char_uuid == BtUuid::uuid128(COMMAND_WRITE) {
                state.command_handle = Some(attr_handle);
                false
            } else if char_uuid == BtUuid::uuid128(IND_CHARACTERISTIC_UUID) {
                state.ind_handle = Some(attr_handle);
                true
            } else {
                false
            }
        };
        if indicate_char {
            self.gatts.add_descriptor(
                service_handle,
                &GattDescriptor {
                    uuid: BtUuid::uuid16(0x2902), // CCCD
                    permissions: enum_set!(Permission::Read | Permission::Write),
                },
            )?;
        }

        Ok(())
    }

    /// Register the CCCD descriptor
    /// Called from within the event callback once we are notified that a descriptor is added,
    fn register_cccd_descriptor(
        &self,
        service_handle: Handle,
        attr_handle: Handle,
        descr_uuid: BtUuid,
    ) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if descr_uuid == BtUuid::uuid16(0x2902) // CCCD UUID
            && state.service_handle == Some(service_handle)
        {
            state.ind_cccd_handle = Some(attr_handle);
        }

        Ok(())
    }

    /// Receive data from a client
    /// Called from within the event callback once we are notified for the connection MTU
    fn register_conn_mtu(&self, conn_id: ConnectionId, mtu: u16) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if let Some(conn) = state
            .connections
            .iter_mut()
            .find(|conn| conn.conn_id == conn_id)
        {
            conn.mtu = Some(mtu);
        }

        Ok(())
    }

    /// Create a new connection
    /// Called from within the event callback once we are notified for a new connection
    fn create_conn(&self, conn_id: ConnectionId, addr: BdAddr) -> Result<(), EspError> {
        let added = {
            let mut state = self.state.lock().unwrap();

            if state.connections.len() < MAX_CONNECTIONS {
                state
                    .connections
                    .push(Connection {
                        peer: addr,
                        conn_id,
                        subscribed: false,
                        mtu: None,
                    })
                    .map_err(|_| ())
                    .unwrap();

                true
            } else {
                false
            }
        };

        if added {
            self.gap.set_conn_params_conf(addr, 10, 20, 0, 400)?;
        }

        Ok(())
    }
    fn delete_conn(&self, addr: BdAddr) -> Result<(), EspError> {
        let mut state = self.state.lock().unwrap();

        if let Some(index) = state
            .connections
            .iter()
            .position(|Connection { peer, .. }| *peer == addr)
        {
            state.connections.swap_remove(index);
        }
        let mut unlocked_is_subscribed = self.is_subscribed.lock().unwrap();
        *unlocked_is_subscribed = false;
        self.gap.start_advertising()?;
        Ok(())
    }

    /// A helper method to process a client sending us data to the "recv" characteristic
    #[allow(clippy::too_many_arguments)]
    fn recv(
        &self,
        _gatt_if: GattInterface,
        conn_id: ConnectionId,
        _trans_id: TransferId,
        addr: BdAddr,
        handle: Handle,
        offset: u16,
        need_rsp: bool,
        is_prep: bool,
        value: &[u8],
    ) -> Result<bool, EspError> {
        let mut state = self.state.lock().unwrap();

        let recv_handle = state.recv_handle;
        let ind_cccd_handle = state.ind_cccd_handle;

        let Some(conn) = state
            .connections
            .iter_mut()
            .find(|conn| conn.conn_id == conn_id)
        else {
            return Ok(false);
        };

        if Some(handle) == ind_cccd_handle {
            // Subscribe or unsubscribe to our indication characteristic

            if offset == 0 && value.len() == 2 {
                let value = u16::from_le_bytes([value[0], value[1]]);
                if value == 0x01 {
                    if !conn.subscribed {
                        conn.subscribed = true;
                        self.on_subscribed(conn.peer);
                    }
                } else if conn.subscribed {
                    conn.subscribed = false;
                    self.on_unsubscribed(conn.peer);
                }
            }
        } else if Some(handle) == recv_handle {
            // Receive data on the recv characteristic

            self.on_recv(addr, value, offset, conn.mtu);
        } else if Some(handle) == state.calib_handle {
            if need_rsp || !is_prep {
                self.on_calib(addr, value, offset, state.calib_handle.unwrap())
            }
        } else if Some(handle) == state.command_handle {
            self.on_command(addr, value, offset, state.command_handle.unwrap())
        } else {
            return Ok(false);
        }

        Ok(true)
    }

    /// A helper method that sends a response to the peer that just sent us some data on the "recv"
    /// characteristic.
    ///
    /// This is only necessary, because we support write confirmation
    /// (which is the more complex case as compared to unconfirmed writes).
    #[allow(clippy::too_many_arguments)]
    fn send_write_response(
        &self,
        gatt_if: GattInterface,
        conn_id: ConnectionId,
        trans_id: TransferId,
        handle: Handle,
        offset: u16,
        need_rsp: bool,
        is_prep: bool,
        value: &[u8],
    ) -> Result<(), EspError> {
        if !need_rsp {
            return Ok(());
        }

        if is_prep {
            let mut state = self.state.lock().unwrap();

            state
                .response
                .attr_handle(handle)
                .auth_req(0)
                .offset(offset)
                .value(value)
                .map_err(|_| EspError::from_infallible::<ESP_FAIL>())?;

            self.gatts.send_response(
                gatt_if,
                conn_id,
                trans_id,
                GattStatus::Ok,
                Some(&state.response),
            )?;
        } else {
            self.gatts
                .send_response(gatt_if, conn_id, trans_id, GattStatus::Ok, None)?;
        }
        Ok(())
    }

    pub fn check_esp_status(&self, status: Result<(), EspError>) {
        if let Err(e) = status {
            warn!("Got status: {e:?}");
        }
    }

    fn check_bt_status(&self, status: BtStatus) -> Result<(), EspError> {
        if !matches!(status, BtStatus::Success) {
            warn!("Got status: {status:?}");
            Err(EspError::from_infallible::<ESP_FAIL>())
        } else {
            Ok(())
        }
    }

    fn check_gatt_status(&self, status: GattStatus) -> Result<(), EspError> {
        if !matches!(status, GattStatus::Ok) {
            warn!("Got status: {status:?}");
            Err(EspError::from_infallible::<ESP_FAIL>())
        } else {
            Ok(())
        }
    }
}
pub fn process_write(
    write_buffers: &RefCell<WriteBufferMap>,
    addr: BdAddr,
    handle: Handle,
    data: &[u8],
    offset: u16,
    end_char: u8,
) -> Option<Vec<u8>> {
    let mut buffers = write_buffers.borrow_mut();
    let buf = buffers.entry((addr.addr(), handle)).or_default();

    // Ensure buffer is large enough for this chunk
    if buf.len() < offset as usize + data.len() {
        buf.resize(offset as usize + data.len(), 0);
    }

    // Copy the chunk into the buffer
    buf[offset as usize..offset as usize + data.len()].copy_from_slice(data);

    if let Some(&last_byte) = buf.last() {
        if last_byte == end_char {
            // Full write received → take the buffer
            return buffers.remove(&(addr.addr(), handle));
        }
    }
    None
}
