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
    CALIBRATE_REF_CHANNEL, CLIENT_CONNECTED, COLOR_REFERENCES_WATCH, DATA_UPDATE_CHANNEL,
    DEVICE_INFO_WATCH, RGB_MULTIPLIERS_WATCH, SETTINGS_DATA_WATCH,
    helpers::{
        RGBMultipliers,
        calibration::{
            CalibrationCommand, ColorReferenceView, ColorReferences, MAX_COLOR_REFERENCES,
            RemoveReferenceCommand, reference_views,
        },
        storage::{Settings, WifiCreds},
    },
    tasks::http::AppState,
};

type ReferenceList = heapless::Vec<ColorReferenceView, MAX_COLOR_REFERENCES>;

pub fn config_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState>
{
    picoserve::Router::new()
        .route("/settings", get(get_settings).post(set_settings))
        .route("/rgb", get(get_rbg_mutlipliers).post(set_rgb_mutlipliers))
        .route("/wifi", post(set_wifi_creds))
        .route("/info", get(get_device_info))
        .route("/color-references", get(get_color_references))
        .route("/color-references/add", post(add_color_reference))
        .route("/color-references/remove", post(remove_color_reference))
        .route("/color-references/clear", post(clear_color_references))
        // older name of /color-references/add
        .route("/auto-calibrate", post(add_color_reference))
}

async fn get_settings() -> impl IntoResponse {
    let mut cfg_recv = SETTINGS_DATA_WATCH.anon_receiver();
    response::Json(cfg_recv.try_get().unwrap())
}

async fn set_settings(extract::Json(settings): extract::Json<Settings>) -> impl IntoResponse {
    DATA_UPDATE_CHANNEL.publish_immediate(crate::DataUpdate::Settings(settings.clone()));
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

fn current_references() -> ColorReferences {
    COLOR_REFERENCES_WATCH.try_get().unwrap_or_default()
}

/// Takes effect right away and is saved in the background
fn store_references(references: ColorReferences) -> response::Json<ReferenceList> {
    COLOR_REFERENCES_WATCH.sender().send(references.clone());
    let views = reference_views(&references.references);
    DATA_UPDATE_CHANNEL.publish_immediate(crate::DataUpdate::ColorReferences(references));
    response::Json(views)
}

async fn get_color_references() -> impl IntoResponse {
    response::Json(reference_views(&current_references().references))
}

/// Adds the inserted filament as a reference. The measurement task does that on its next pass,
/// which only happens with a filament in the chamber, otherwise the request times out.
async fn add_color_reference<'a>(
    extract::Json(calib_d): extract::Json<CalibrationCommand>,
) -> Result<response::Json<ReferenceList>, (StatusCode, &'a str)> {
    let client_connected = CLIENT_CONNECTED.try_get().unwrap_or(false);
    if !client_connected {
        return Err((
            StatusCode::PRECONDITION_REQUIRED,
            "client needs to be listening and connected",
        ));
    }
    if current_references().references.is_full() {
        return Err((
            StatusCode::CONFLICT,
            "all reference slots are used, remove one first",
        ));
    }

    let mut references_recv = COLOR_REFERENCES_WATCH.anon_receiver();
    // A new receiver starts at message id 0, so try_changed() would immediately return the
    // references from before. Mark the current ones as seen first.
    let _ = references_recv.try_get();
    CALIBRATE_REF_CHANNEL.publish_immediate(calib_d);
    let changed_fut = async {
        loop {
            if let Some(d) = references_recv.try_changed() {
                return d;
            }
            Timer::after_millis(50).await
        }
    };
    let timeout_fut = Timer::after_secs(5);
    match select(changed_fut, timeout_fut).await {
        Either::First(d) => Ok(response::Json(reference_views(&d.references))),
        Either::Second(_) => Err((StatusCode::REQUEST_TIMEOUT, "no filament inserted")),
    }
}

async fn remove_color_reference(
    extract::Json(cmd): extract::Json<RemoveReferenceCommand>,
) -> impl IntoResponse {
    let mut references = current_references();
    if (cmd.index as usize) < references.references.len() {
        references.references.remove(cmd.index as usize);
    }
    store_references(references)
}

async fn clear_color_references() -> impl IntoResponse {
    store_references(ColorReferences::default())
}
