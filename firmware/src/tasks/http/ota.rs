use picoserve::{extract, request};

use crate::tasks::http::AppState;

pub fn ota_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::new()
}

async fn upload_firmware(req: request::Request<>)
