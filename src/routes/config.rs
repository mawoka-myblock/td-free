use std::{borrow::Cow, collections::HashMap};

use edge_http::io::Error as EdgeError;
use edge_http::io::server::Connection;
use embedded_io_async::{Read, Write};
use esp_idf_svc::hal::reset;
use log::error;
use picoserve::response::{Body, HeadersIter, IntoResponse, Response, ResponseWriter, StatusCode};
use url::Url;

use crate::{
    AppState, WsHandler, WsHandlerError,
    helpers::nvs::{
        get_saved_algorithm_variables, read_spoolman_data, save_algorithm_variables,
        save_spoolman_data,
    },
    routes::serve::{serve_algo_setup_page, serve_wifi_setup_page},
    wifi,
};

pub async fn read_config_route(state: AppState) -> Response<impl HeadersIter, impl Body> {
    let spoolman_available = read_spoolman_data(state.nvs.as_ref().clone()).0.is_some();
    let color_available = state.rgb.is_some();
    let version = option_env!("TD_FREE_VERSION").unwrap_or("UNKNOWN");
    let data = format!(
        r#"{{"spoolman_available": {spoolman_available}, "color_available": {color_available},"version": "{version}"}}"#,
    );
    Response::new(StatusCode::OK, data).with_header("Content-Type", "application/json")
}

#[derive(serde::Deserialize)]
pub struct WifiRouteParams {
    pub ssid: Option<String>,
    pub password: Option<String>,
}

pub async fn wifi_route(
    state: AppState,
    params: WifiRouteParams,
) -> Response<impl HeadersIter, impl Body> {
    if params.ssid.is_none() && params.password.is_none() {
        let saved_ssid =
            wifi::get_wifi_ssid(state.nvs.clone().as_ref().clone()).unwrap_or_default();
        return Response::new(StatusCode::OK, serve_wifi_setup_page(&saved_ssid, ""))
            .with_header("Content-Type", "text/html");
    }
    if params.ssid.is_none() {
        return Response::new(StatusCode::OK, serve_wifi_setup_page("", "SSID is not set"))
            .with_header("Content-Type", "text/html");
    }
    if params.password.is_none() {
        return Response::new(
            StatusCode::OK,
            serve_wifi_setup_page("", "Password is not set"),
        )
        .with_header("Content-Type", "text/html");
    }
    match wifi::save_wifi_creds(
        &params.ssid.unwrap(),
        &params.password.unwrap(),
        state.nvs.clone().as_ref().clone(),
    ) {
        Ok(_) => {
            embassy_time::Timer::after_millis(50).await;
            reset::restart();
        }
        Err(e) => {
            error!("{e:?}");
            embassy_time::Timer::after_millis(50).await;
            reset::restart();
        }
    };
}

#[derive(serde::Deserialize)]
pub struct AlgoQueryParams {
    pub m: Option<String>,
    pub b: Option<String>,
    pub threshold: Option<String>,
    pub spoolman_url: Option<String>,
    pub spoolman_field_name: Option<String>,
}

pub async fn algorithm_route(
    state: AppState,
    params: AlgoQueryParams,
) -> Response<impl HeadersIter, impl Body> {
    if params.m.is_none()
        && params.b.is_none()
        && params.threshold.is_none()
        && params.spoolman_url.is_none()
    {
        let saved_algorithm = get_saved_algorithm_variables(state.nvs.as_ref().clone());
        let saved_spoolman = read_spoolman_data(state.nvs.as_ref().clone());
        let spoolman_url = match saved_spoolman.0 {
            Some(d) => d,
            None => "".to_string(),
        };
        let spoolman_field_name = match saved_spoolman.1 {
            Some(d) => d,
            None => "td".to_string(),
        };
        return Response::new(
            StatusCode::OK,
            serve_algo_setup_page(
                saved_algorithm.b,
                saved_algorithm.m,
                saved_algorithm.threshold,
                &spoolman_url,
                &spoolman_field_name,
            ),
        )
        .with_header("Content-Type", "text/html");
    }
    let mod_b_value = params
        .b
        .as_deref()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("0.0".to_string()));
    let mod_m_value = params
        .m
        .as_deref()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("1.0".to_string()));
    let mod_threshold_value = params
        .threshold
        .as_deref()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("0.5".to_string()));
    let mod_spoolman_value = params
        .spoolman_url
        .as_deref()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("".to_string()));
    let mod_spoolman_field_name = params
        .spoolman_field_name
        .as_deref()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned("".to_string()));
    let save_spoolman_res = save_spoolman_data(
        &mod_spoolman_value,
        &mod_spoolman_field_name,
        state.nvs.as_ref().clone(),
    );
    if save_spoolman_res.is_err() {
        error!("{:?}", save_spoolman_res.err().unwrap());
        embassy_time::Timer::after_millis(50).await;
        reset::restart();
    }
    match save_algorithm_variables(
        &mod_b_value,
        &mod_m_value,
        &mod_threshold_value,
        state.nvs.as_ref().clone(),
    ) {
        Ok(_) => {
            return Response::new(
                StatusCode::OK,
                serve_algo_setup_page(
                    mod_b_value.parse::<f32>().unwrap_or(0.0),
                    mod_m_value.parse::<f32>().unwrap_or(1.0),
                    mod_threshold_value.parse::<f32>().unwrap_or(0.5),
                    &mod_spoolman_value,
                    &mod_spoolman_field_name,
                ),
            )
            .with_header("Content-Type", "text/html");
        }
        Err(e) => {
            error!("{e:?}");
            embassy_time::Timer::after_millis(50).await;
            reset::restart();
        }
    };
    // Ok(())
}
