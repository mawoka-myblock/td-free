use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_sync::pubsub::PubSubBehavior;
use embassy_time::Timer;
use picoserve::{
    extract,
    response::{self, IntoResponse, StatusCode},
    routing::{get, post},
};

use crate::{
    CALIBRATE_REF_CHANNEL, CLIENT_CONNECTED, DATA_UPDATE_CHANNEL, DEVICE_INFO_WATCH,
    RGB_MULTIPLIERS_WATCH, SETTINGS_DATA_WATCH,
    helpers::{
        RGBMultipliers,
        calibration::CalibrationCommand,
        storage::{Settings, WifiCreds},
    },
    tasks::http::AppState,
};

pub fn config_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState>
{
    picoserve::Router::new()
        .route("/settings", get(get_settings).post(set_settings))
        .route("/rgb", get(get_rbg_mutlipliers).post(set_rgb_mutlipliers))
        .route("/wifi", post(set_wifi_creds))
        .route("/info", get(get_device_info))
        .route("/auto-calibrate", post(set_auto_calibrate))
}

async fn get_settings() -> impl IntoResponse {
    let mut cfg_recv = SETTINGS_DATA_WATCH.anon_receiver();
    response::Json(cfg_recv.try_get().unwrap())
}

async fn set_settings(extract::Json(settings): extract::Json<Settings>) -> impl IntoResponse {
    DATA_UPDATE_CHANNEL.publish_immediate(crate::DataUpdate::Settings(settings));
    response::Json(settings)
}

async fn get_rbg_mutlipliers() -> impl IntoResponse {
    let mut rgb_recv = RGB_MULTIPLIERS_WATCH.anon_receiver();
    response::Json(rgb_recv.try_get().unwrap())
}

async fn set_rgb_mutlipliers(
    extract::Json(rgb_m): extract::Json<RGBMultipliers>,
) -> impl IntoResponse {
    DATA_UPDATE_CHANNEL.publish_immediate(crate::DataUpdate::RgbMulti(rgb_m));
    response::Json(rgb_m)
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

async fn set_auto_calibrate<'a>(
    extract::Json(calib_d): extract::Json<CalibrationCommand>,
) -> Result<response::Json<RGBMultipliers>, (StatusCode, &'a str)> {
    let client_connected = CLIENT_CONNECTED.try_get().unwrap_or(false);
    if !client_connected {
        return Err((
            StatusCode::PRECONDITION_REQUIRED,
            "client needs to be listening and connected",
        ));
    }

    CALIBRATE_REF_CHANNEL.publish_immediate(calib_d);
    let mut rgb_multi_recv = RGB_MULTIPLIERS_WATCH.anon_receiver();
    let changed_fut = async {
        loop {
            if let Some(d) = rgb_multi_recv.try_changed() {
                return d;
            }
            Timer::after_millis(50).await
        }
    };
    let timeout_fut = Timer::after_secs(5);
    return match select(changed_fut, timeout_fut).await {
        Either::First(d) => Ok(response::Json(d)),
        Either::Second(_) => {
            // timeout occurred
            Err((StatusCode::REQUEST_TIMEOUT, "fn timeouted"))
        }
    };
}
