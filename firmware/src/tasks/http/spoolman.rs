use embassy_sync::pubsub::PubSubBehavior;
use embassy_time::Timer;
use heapless::String;
use picoserve::extract;
use picoserve::io::Write;
use picoserve::response::custom::{CustomBody, CustomResponse};
use picoserve::response::{IntoResponse, NoContent, StatusCode};
use picoserve::routing::{get, post};
use serde::Deserialize;

use crate::SETTINGS_DATA_WATCH;
use crate::tasks::{
    http::AppState,
    spoolman::{SPOOLMAN_CHANNEL, SPOOLMAN_PIPE, SpoolmanCommands},
};

pub fn spoolman_router()
-> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::new()
        .route("/get-filaments", get(spoolman_handler))
        .route("/set-filament", post(set_filament_td))
}

pub struct SpoolmanBody;

impl CustomBody for SpoolmanBody {
    async fn write_response_body<W>(self, mut writer: W) -> Result<(), W::Error>
    where
        W: Write,
    {
        let mut buffer = [0u8; 2048];
        SPOOLMAN_CHANNEL.publish_immediate(SpoolmanCommands::Get);
        let mut chan_recv = SPOOLMAN_CHANNEL.subscriber().unwrap();

        loop {
            if SPOOLMAN_PIPE.is_empty() {
                Timer::after_millis(2).await;
                if chan_recv.try_next_message_pure() == Some(SpoolmanCommands::Done) {
                    break;
                }
            }
            let n = SPOOLMAN_PIPE.read(&mut buffer).await;
            writer.write_all(&buffer[..n]).await?;
            writer.flush().await?;
        }
        Ok(())
    }
}

pub enum SpoolmanResponseBody {
    Empty,
    Spoolman,
}

impl CustomBody for SpoolmanResponseBody {
    async fn write_response_body<W>(self, writer: W) -> Result<(), W::Error>
    where
        W: Write,
    {
        match self {
            Self::Empty => Ok(()),
            Self::Spoolman => SpoolmanBody.write_response_body(writer).await,
        }
    }
}

// -----------------------------------------------------------------------------
// Picoserve handler
// -----------------------------------------------------------------------------

pub async fn spoolman_handler() -> impl IntoResponse {
    let spoolman_set_up = match SETTINGS_DATA_WATCH.anon_receiver().try_get() {
        Some(d) => d.spoolman_host.is_some() && d.spoolman_port.is_some(),
        None => false,
    };
    if !spoolman_set_up {
        return CustomResponse::build(StatusCode::PRECONDITION_FAILED)
            .with_header("Content-Type", "application/text")
            .with_header("Access-Control-Allow-Origin", "*")
            .with_body(SpoolmanResponseBody::Empty);
    }
    CustomResponse::build(StatusCode::OK)
        .with_header("Content-Type", "application/json")
        .with_header("Access-Control-Allow-Origin", "*")
        .with_body(SpoolmanResponseBody::Spoolman)
}

#[derive(Debug, Deserialize)]
struct SetTdFilament {
    filament_id: u32,
    td: Option<f32>,
    color: Option<String<6>>,
}

async fn set_filament_td(extract::Json(data): extract::Json<SetTdFilament>) -> impl IntoResponse {
    let spoolman_set_up = match SETTINGS_DATA_WATCH.anon_receiver().try_get() {
        Some(d) => d.spoolman_host.is_some() && d.spoolman_port.is_some(),
        None => false,
    };
    if !spoolman_set_up {
        return (StatusCode::PRECONDITION_FAILED, NoContent);
    }
    SPOOLMAN_CHANNEL.publish_immediate(SpoolmanCommands::Set(
        data.filament_id,
        data.td,
        data.color,
    ));
    let mut sub = SPOOLMAN_CHANNEL.subscriber().unwrap();
    let resp_code = loop {
        let msg = sub.next_message_pure().await;
        match msg {
            SpoolmanCommands::SetResponse(code) => break code,
            _ => continue,
        };
    };
    (StatusCode::new(resp_code), NoContent)
}
