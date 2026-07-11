use defmt::info;
use embassy_sync::pubsub::PubSubBehavior;
use picoserve::{
    extract,
    response::{self, IntoResponse},
    routing::{get, post},
};

use crate::{
    DATA_UPDATE_CHANNEL, DEVICE_INFO_WATCH, SETTINGS_DATA_WATCH,
    helpers::storage::{Settings, WifiCreds},
    tasks::http::AppState,
};

pub fn config_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState>
{
    picoserve::Router::new()
        .route("/settings", get(get_settings).post(set_settings))
        .route("/wifi", post(set_wifi_creds))
        .route("/info", get(get_device_info))
}

async fn get_settings() -> impl IntoResponse {
    let mut cfg_recv = SETTINGS_DATA_WATCH.anon_receiver();
    response::Json(cfg_recv.try_get().unwrap())
}

async fn set_settings(extract::Json(settings): extract::Json<Settings>) -> impl IntoResponse {
    DATA_UPDATE_CHANNEL.publish_immediate(crate::DataUpdate::Settings(settings));
    response::Json(settings)
}

async fn set_wifi_creds(extract::Json(wifi_creds): extract::Json<WifiCreds>) -> impl IntoResponse {
    info!("Received wifi creds: {}", wifi_creds);
    DATA_UPDATE_CHANNEL.publish_immediate(crate::DataUpdate::Wifi(wifi_creds.clone()));
    response::Json(wifi_creds)
}

async fn get_device_info() -> impl IntoResponse {
    let mut info_recv = DEVICE_INFO_WATCH.anon_receiver();
    response::Json(info_recv.try_get().unwrap())
}
