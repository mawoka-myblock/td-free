mod frontend;
pub mod network;
mod sse;
use picoserve::AppRouter;
use picoserve::response::{IntoResponse, StatusCode};
use picoserve::{AppBuilder, routing::PathRouter};

pub const WEB_TASK_POOL_SIZE: usize = 2;

static CONFIG: picoserve::Config = picoserve::Config::const_default().keep_connection_alive();

pub struct AppState {}
pub struct AppProps {
    state: AppState,
}

impl AppProps {
    pub fn new() -> Self {
        Self { state: AppState {} }
    }
}

impl Default for AppProps {
    fn default() -> Self {
        Self::new()
    }
}

#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
pub async fn web_task(
    task_id: usize,
    stack: embassy_net::Stack<'static>,
    app: &'static AppRouter<AppProps>,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 2048]; // was 1024
    let mut tcp_tx_buffer = [0; 2048];
    let mut http_buffer = [0; 2048];

    picoserve::Server::new(app, &CONFIG, &mut http_buffer)
        .listen_and_serve(task_id, stack, port, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await
        .into_never()
}

impl AppBuilder for AppProps {
    type PathRouter = impl PathRouter;

    fn build_app(self) -> picoserve::Router<Self::PathRouter> {
        let Self { state } = self;

        picoserve::Router::new()
            .nest("", frontend::frontend_router())
            .nest("/events", sse::event_router())
            // .nest("/api", api::api_router())
            .with_state(state)
    }
}

pub struct RedirectToPortal;

impl picoserve::routing::PathRouterService<AppState> for RedirectToPortal {
    async fn call_path_router_service<
        R: picoserve::io::Read,
        W: picoserve::response::ResponseWriter<Error = R::Error>,
    >(
        &self,
        _state: &AppState,
        _path_parameters: (),
        _path: picoserve::request::Path<'_>,
        request: picoserve::request::Request<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        (
            StatusCode::FOUND,
            // [("Content-Encoding", "gzip"), ("Content-Type", "text/html")],
            [("Location", "http://10.10.10.1/")],
            b"".as_slice(),
        )
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}
