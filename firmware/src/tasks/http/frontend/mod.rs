use picoserve::{
    response::{IntoResponse, StatusCode},
    routing::get,
};

use crate::tasks::http::AppState;

pub fn frontend_router()
-> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::new().route("/", get(get_index))
    // .route("/app.js", get(get_js))
    // .route("/app.css", get(get_css))
}

async fn get_index() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("Content-Encoding", "gzip"), ("Content-Type", "text/html")],
        b"hallo".as_slice(),
    )
}
