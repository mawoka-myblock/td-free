use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_time::Timer;
use picoserve::{
    extract::Json,
    response::{self, EventStream},
    routing::get,
};

use crate::{CLIENT_CONNECTED, MEASUREMENT_DATA, tasks::http::AppState};
pub fn event_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState>
{
    picoserve::Router::new().route("/data", get(async move || EventStream(Events)))
}

struct Events;
impl response::sse::EventSource for Events {
    async fn write_events<W: picoserve::io::Write>(
        self,
        mut writer: response::sse::EventWriter<'_, W>,
    ) -> Result<(), W::Error> {
        // Create a guard that will set CLIENT_CONNECTED to false when dropped
        struct ClientConnectedGuard;
        impl Drop for ClientConnectedGuard {
            fn drop(&mut self) {
                info!("Client disconnected!");
                CLIENT_CONNECTED.sender().send_if_modified(|v| {
                    let changed = *v != Some(false);
                    *v = Some(false);
                    changed
                });
            }
        }
        let _guard = ClientConnectedGuard;

        // Subscribe to measurement data changes
        let mut measurement_rx = MEASUREMENT_DATA
            .receiver()
            .expect("Couldn't get a new receiver for measurement data");

        // Set client connected to true on connection
        CLIENT_CONNECTED.sender().send_if_modified(|v| {
            let changed = *v != Some(true);
            *v = Some(true);
            changed
        });

        loop {
            match select(measurement_rx.changed(), Timer::after_secs(15)).await {
                Either::First(d) => {
                    if d.is_some() {
                        writer.write_event("measurement_changed", Json(d)).await?
                    } else {
                        writer
                            .write_event("measurement_changed", "no_filament")
                            .await?
                    }
                }
                Either::Second(_) => writer.write_keepalive().await?,
            }
        }
    }
}
