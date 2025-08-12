pub mod config;
pub mod rgb;
pub mod serve;

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    str,
};

use edge_http::Method as EdgeMethod;
use edge_http::io::server::{Connection, Handler};
use embedded_io_async::{Read, Write};
use embedded_svc::http::client::Client;
use esp_idf_svc::{
    http::{Method, client::EspHttpConnection},
    io::Write as _,
};
use picoserve::{
    AppWithStateBuilder,
    extract::{Json, Query, State},
    response::{Body, HeadersIter, Response, StatusCode},
    routing::{PathRouter, get, post},
};
use url::Url;

use crate::{
    AppProps, AppState, EdgeError, WsHandler, WsHandlerError,
    helpers::nvs::read_spoolman_data,
    routes::{
        config::{
            AlgoQueryParams, WifiRouteParams, algorithm_route, read_config_route, wifi_route,
        },
        rgb::{
            AutoCalibrateGrayInput, SetRgbMultiplierJsonData, auto_calibrate_gray_reference,
            get_rgb_multipliers, set_rgb_multipliers,
        },
    },
    wifi::WifiEnum,
};

static INDEX_HTML: &str = include_str!("static/index.html");
static STYLE_CSS: &str = include_str!("static/style.css");
static SCRIPT_JS: &str = include_str!("static/script.js");
static SCRIPT_CALIBRATE_JS: &str = include_str!("static/calibrate/script.js");
static CALIBRATE_HTML: &str = include_str!("static/calibrate/index.html");

impl WsHandler {
    /*
       pub async fn spoolman_get_filaments<T, const N: usize>(
           &self,
           conn: &mut Connection<'_, T, N>,
       ) -> Result<(), WsHandlerError<EdgeError<T::Error>, edge_ws::Error<T::Error>>>
       where
           T: Read + Write,
       {
           let spoolman_url = read_spoolman_url(self.nvs.as_ref().clone());
           if spoolman_url.is_none() {
               conn.initiate_response(400, None, &[("Content-Type", "application/json")])
                   .await?;
               conn.write_all(r#"{"status": "spoolman_url_not_set", "filaments": []}"#.as_ref())
                   .await?;
               return Ok(());
           }
           let mut client = Client::wrap(EspHttpConnection::new(&Default::default()).unwrap());
           let url = format!("{}/api/v1/filament", spoolman_url.unwrap());
           let req = client
               .request(Method::Get, &url, &[("accept", "application/json")])
               .unwrap();
           let res = req.submit();
           if res.is_err() {
               conn.initiate_response(500, None, &[("Content-Type", "application/json")])
                   .await?;
               conn.write_all(r#"{"status": "request_to_spoolman_failed", "filaments": []}"#.as_ref())
                   .await?;
               return Ok(());
           }
           let mut res = res.unwrap();
           let mut buf = [0u8; 4048];
           let _ = res.read(&mut buf);
           info!("Response: {}", String::from_utf8_lossy(&buf));
           let base_value: Value = serde_json::from_slice::<Value>(&buf).unwrap();
           let stream = base_value.as_array().unwrap();
           conn.initiate_response(200, None, &[("Content-Type", "application/json")])
               .await?;
           conn.write_all(r#"{"status": "request_to_spoolman_failed", "filaments": ["#.as_ref())
               .await?;
           for (i, value) in stream.iter().enumerate() {
               let mut data = format!(
                   r#"{{"name": "{}", "id": {}}}"#,
                   value.get("name").unwrap().as_str().unwrap(),
                   value.get("id").unwrap().as_i64().unwrap()
               );
               if i != 0 {
                   data = ",".to_string() + &data
               }
               conn.write_all(data.as_ref()).await?;
           }
           conn.write_all("]}".as_ref()).await?;
           return Ok(());
       }
    */
    pub async fn spoolman_set_filament<T, const N: usize>(
        &self,
        path: &str,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), WsHandlerError<EdgeError<T::Error>>>
    where
        T: Read + Write,
    {
        if *self.wifi_status.lock().unwrap() != WifiEnum::Connected {
            conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Not connected to station, Spoolman unavailable."#.as_ref())
                .await?;
            return Ok(());
        }
        let url = Url::parse(&format!("http://google.com{path}")).unwrap();
        let url_params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let value = url_params.get("value");
        let filament_id = url_params.get("filament_id");
        if filament_id.is_none() || value.is_none() {
            conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Filament ID and/or Value are unset."#.as_ref())
                .await?;
            return Ok(());
        }
        let value: f32 = match value.unwrap().parse::<f32>() {
            Ok(d) => d,
            Err(_) => {
                conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                    .await?;
                conn.write_all(r#"Value is not an integer."#.as_ref())
                    .await?;
                return Ok(());
            }
        };
        let filament_id: i32 = match filament_id.unwrap().parse::<i32>() {
            Ok(d) => d,
            Err(_) => {
                conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                    .await?;
                conn.write_all(r#"Filament ID is not an integer."#.as_ref())
                    .await?;
                return Ok(());
            }
        };
        let spoolman_data = read_spoolman_data(self.nvs.as_ref().clone());
        if spoolman_data.0.is_none() || spoolman_data.0.clone().unwrap().is_empty() {
            conn.initiate_response(400, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Could not read storage."#.as_ref())
                .await?;
            return Ok(());
        }

        let mut client = Client::wrap(EspHttpConnection::new(&Default::default()).unwrap());
        let url = format!(
            "{}/api/v1/filament/{}",
            spoolman_data.0.unwrap(),
            filament_id
        );
        let payload = format!(
            r#"{{"extra": {{"{}": "{}"}}}}"#,
            spoolman_data.1.unwrap_or("td".to_string()),
            value
        );
        let payload_length = format!("{}", payload.len());
        let headers = [
            ("accept", "application/json"),
            ("content-type", "application/json"),
            ("content-length", &payload_length),
        ];
        let mut req = client.request(Method::Patch, &url, &headers).unwrap();
        req.write_all(payload.as_ref()).unwrap();
        req.flush().unwrap();
        let res = req.submit();
        if res.is_err() {
            conn.initiate_response(500, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Request to Spoolman failed!"#.as_ref())
                .await?;
            return Ok(());
        }
        let res = res.unwrap();
        if res.status() != 200 {
            conn.initiate_response(500, None, &[("Content-Type", "text/plain")])
                .await?;
            conn.write_all(r#"Spoolman did not reply with 200"#.as_ref())
                .await?;
            return Ok(());
        }
        conn.initiate_response(302, None, &[("Location", "/")])
            .await?;

        Ok(())
    }
}

pub async fn fallback_route(state: AppState) -> Response<impl HeadersIter, impl Body> {
    // Try to acquire the BUSY lock without blocking
    state.ext_channel.send(None).await;
    embassy_time::Timer::after_millis(100).await;
    let data = state.ext_channel.receive().await.unwrap_or_default();
    return Response::new(StatusCode::OK, data).with_header("Content-Type", "text/raw");
}

pub async fn get_router() -> picoserve::Router<impl PathRouter<AppState>, AppState> {
    picoserve::Router::new()
        .route(
            "/",
            get(|| async move {
                Response::new(StatusCode::OK, INDEX_HTML).with_header("Content-Type", "text/html")
            }),
        )
        .route(
            "/style.css",
            get(|| async move {
                Response::new(StatusCode::OK, STYLE_CSS).with_header("Content-Type", "text/css")
            }),
        )
        .route(
            "/script.js",
            get(|| async move {
                Response::new(StatusCode::OK, SCRIPT_JS)
                    .with_header("Content-Type", "application/javascript")
            }),
        )
        .route(
            "/calibrate/script.js",
            get(|| async move {
                Response::new(StatusCode::OK, SCRIPT_CALIBRATE_JS)
                    .with_header("Content-Type", "application/javascript")
            }),
        )
        .route(
            "/calibrate",
            get(|| async move {
                Response::new(StatusCode::OK, CALIBRATE_HTML)
                    .with_header("Content-Type", "text/html")
            }),
        )
        .route(
            "/settings",
            get(
                |State(state): State<AppState>, Query(query): Query<AlgoQueryParams>| async move {
                    // algorithm_route(state, query)
                    algorithm_route(state, query).await
                },
            ), // TODO
        )
        .route(
            "/wifi",
            get(
                |State(state): State<AppState>, Query(query): Query<WifiRouteParams>| async move {
                    wifi_route(state, query).await
                },
            ), // TODO
        )
        .route(
            "/falback",
            get(|State(state): State<AppState>| async move { fallback_route(state).await }), // TODO
        )
        // .route(
        //     "/spoolman/set",
        //     get(|State(state): State<AppState>| async move { "Hello World" }), // TODO
        // )
        .route(
            "/rgb_multipliers",
            get(|State(state): State<AppState>| async move { get_rgb_multipliers(state).await }), // TODO
        )
        .route(
            "/rgb_multipliers",
            post(|State(state): State<AppState>, Json(data): Json<SetRgbMultiplierJsonData>| async move { set_rgb_multipliers(state, data).await }), // TODO
        )
        .route(
            "/auto_calibrate",
            get(|State(state): State<AppState>, Json(data): Json<AutoCalibrateGrayInput>| async move { auto_calibrate_gray_reference(state, data).await }), // TODO
        )
        .route(
            "/config",
            get(|State(state): State<AppState>| async move { read_config_route(state).await }), // TODO
        )
}
