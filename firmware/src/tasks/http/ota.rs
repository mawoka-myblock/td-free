use core::sync::atomic::Ordering;

use defmt::{info, warn};
use embassy_time::{Duration, Timer};
use embedded_io_async::Read as AsyncRead;
use embedded_storage::Storage;
use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::PARTITION_TABLE_MAX_LEN;
use esp_hal::peripherals::FLASH;
use esp_hal::system::software_reset;
use esp_storage::FlashStorage;
use picoserve::io::Read;
use picoserve::request::{Request, RequestBodyConnection};
use picoserve::response::{IntoResponse, ResponseWriter, StatusCode};
use picoserve::routing::RequestHandlerService;

use crate::tasks::http::AppState;

/// Buffer size used when reading one HTTP body chunk from the network.
const READ_BUF_LEN: usize = 1024;

/// Reused static buffer for the OTA updater so it does not live on the web task stack.
static mut OTA_PARTITION_BUF: [u8; PARTITION_TABLE_MAX_LEN] = [0u8; PARTITION_TABLE_MAX_LEN];

/// How long to wait for the background state task to finish a flash operation
/// before giving up.
const FLASH_BUSY_TIMEOUT_MS: usize = 10_000;
const FLASH_BUSY_POLL_MS: u64 = 25;

pub fn ota_router() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::new().route("/upload", picoserve::routing::post_service(OtaUpload))
}

pub struct OtaUpload;

impl RequestHandlerService<AppState> for OtaUpload {
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &AppState,
        _path_parameters: (),
        mut request: Request<'_, R>,
        response_writer: W,
    ) -> Result<picoserve::ResponseSent, W::Error> {
        if request.parts.method() != "POST" {
            return respond(
                StatusCode::METHOD_NOT_ALLOWED,
                "Use POST",
                request,
                response_writer,
            )
            .await;
        }

        // Serialize uploads so only one is running at a time.
        if crate::OTA_IN_PROGRESS.swap(true, Ordering::AcqRel) {
            return respond(
                StatusCode::LOCKED,
                "OTA already in progress",
                request,
                response_writer,
            )
            .await;
        }

        // Wait for any pending NVS operation to finish, then we own the flash.
        let mut ready = false;
        for _ in 0..(FLASH_BUSY_TIMEOUT_MS / FLASH_BUSY_POLL_MS as usize) {
            if !crate::FLASH_LOCKED.load(Ordering::Acquire) {
                ready = true;
                break;
            }
            Timer::after_millis(FLASH_BUSY_POLL_MS).await;
        }
        if !ready {
            warn!("Flash remained busy while starting OTA upload");
            crate::OTA_IN_PROGRESS.store(false, Ordering::Release);
            return respond(
                StatusCode::SERVICE_UNAVAILABLE,
                "Flash busy",
                request,
                response_writer,
            )
            .await;
        }

        let result = {
            let body_conn = &mut request.body_connection;
            upload_firmware(body_conn).await
        };

        let (status, body) = match result {
            Ok(written) => {
                info!("OTA image written ({} bytes), rebooting in 3s", written);
                (StatusCode::OK, "OTA upload complete, rebooting...")
            }
            Err(e) => {
                warn!("OTA upload failed: {}", e);
                crate::OTA_IN_PROGRESS.store(false, Ordering::Release);
                (StatusCode::INTERNAL_SERVER_ERROR, e)
            }
        };

        let conn = request.body_connection.finalize().await?;
        let response_sent = (status, body).write_to(conn, response_writer).await?;

        if result.is_ok() {
            Timer::after(Duration::from_secs(3)).await;
            software_reset();
        }

        Ok(response_sent)
    }
}

async fn upload_firmware<R: Read>(
    body_conn: &mut RequestBodyConnection<'_, R>,
) -> Result<usize, &'static str> {
    let content_length = body_conn.content_length();
    if content_length == 0 {
        return Err("missing Content-Length");
    }

    let mut flash = FlashStorage::new(unsafe { FLASH::steal() });
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(OTA_PARTITION_BUF) };
    let mut ota =
        OtaUpdater::new(&mut flash, buf).map_err(|_| "failed to initialize OTA updater")?;

    let part_size = {
        let (part, _) = ota
            .next_partition()
            .map_err(|_| "no OTA partition available")?;
        part.partition_size()
    };

    if content_length > part_size {
        return Err("firmware image too large for partition");
    }

    let mut reader = body_conn.body().reader();
    let mut read_buf = [0u8; READ_BUF_LEN];
    let mut written: usize = 0;

    info!("Starting OTA upload, {} bytes", content_length);

    loop {
        let n = match reader.read(&mut read_buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return Err("network read failed"),
        };

        if written + n > part_size {
            return Err("firmware image exceeded partition size");
        }

        let (mut part, _) = ota.next_partition().map_err(|_| "OTA partition error")?;
        part.write(written as u32, &read_buf[..n])
            .map_err(|_| "flash write failed")?;

        written += n;
    }

    if written != content_length {
        return Err("upload size mismatch");
    }

    ota.activate_next_partition()
        .map_err(|_| "failed to activate new partition")?;
    ota.set_current_ota_state(OtaImageState::PendingVerify)
        .map_err(|_| "failed to mark image as pending verify")?;

    info!("OTA complete, {} bytes written", written);
    Ok(written)
}

async fn respond<R: Read, W: ResponseWriter<Error = R::Error>>(
    status: StatusCode,
    body: &str,
    request: Request<'_, R>,
    response_writer: W,
) -> Result<picoserve::ResponseSent, W::Error> {
    let conn = request.body_connection.finalize().await?;
    (status, body).write_to(conn, response_writer).await
}
