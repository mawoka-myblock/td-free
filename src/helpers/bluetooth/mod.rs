use std::sync::Arc;

use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};
use embedded_hal_async::delay::DelayNs;
use esp_idf_svc::{
    hal::{delay::FreeRtos, modem::Modem},
    nvs::{EspNvsPartition, NvsDefault},
};

use esp_idf_svc::bt::ble::gap::EspBleGap;
use esp_idf_svc::bt::ble::gatt::server::EspGatts;
use esp_idf_svc::bt::{Ble, BtDriver};
use log::info;

use crate::helpers::bluetooth::server::RunData;

pub mod calib;
pub mod server;

pub async fn init_bt(
    modem: Modem,
    nvs: EspNvsPartition<NvsDefault>,
    data: RunData,
    ext_channel: Arc<Channel<NoopRawMutex, Option<String>, 1>>,
) -> anyhow::Result<()> {
    let bt: Arc<BtDriver<Ble>> = Arc::new(BtDriver::new(modem, Some(nvs.clone()))?);
    let server = server::BtServer::new(
        Arc::new(EspBleGap::new(bt.clone()).unwrap()),
        Arc::new(EspGatts::new(bt.clone()).unwrap()),
        data,
    );
    let gap_server = server.clone();
    server
        .gap
        .subscribe(move |event| {
            gap_server.check_esp_status(gap_server.on_gap_event(event));
        })
        .unwrap();

    let gatts_server = server.clone();

    server
        .gatts
        .subscribe(move |(gatt_if, event)| {
            gatts_server.check_esp_status(gatts_server.on_gatts_event(gatt_if, event))
        })
        .unwrap();

    info!("BLE Gap and Gatts subscriptions initialized");

    server.gatts.register_app(APP_ID).unwrap();

    info!("Gatts BTP app registered");

    let mut ind_data = 0_u16;

    loop {
        ext_channel.send(None).await;
        embassy_time::Timer::after_millis(100).await;
        let data = ext_channel.receive().await.unwrap_or_default();

        server.notify(data.as_bytes()).unwrap();
        ind_data = ind_data.wrapping_add(1);
        embassy_time::Timer::after_millis(1000).await;
    }
}

pub type ExBtDriver = BtDriver<'static, Ble>;
pub type ExEspBleGap = Arc<EspBleGap<'static, Ble, Arc<ExBtDriver>>>;
pub type ExEspGatts = Arc<EspGatts<'static, Ble, Arc<ExBtDriver>>>;
pub const MAX_CONNECTIONS: usize = 2;
pub const APP_ID: u16 = 0;

// Our service UUID
pub const SERVICE_UUID: u128 = 0xad91b201734740479e173bed82d75f9d;

/// Our "recv" characteristic - i.e. where clients can send data.
pub const RECV_CHARACTERISTIC_UUID: u128 = 0xb6fccb5087be44f3ae22f85485ea42c4;
/// Our "indicate" characteristic - i.e. where clients can receive data if they subscribe to it
pub const IND_CHARACTERISTIC_UUID: u128 = 0x503de214868246c4828fd59144da41bf;
pub const CALIB_CHARACTERISTIC_UUID: u128 = 0x11223344556677889900aabbccddeeff;
