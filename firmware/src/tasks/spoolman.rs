use core::fmt::Write;
use core::str::FromStr;

use embassy_net::{Ipv4Address, Stack, tcp::TcpSocket};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, pipe::Pipe, pubsub::PubSubChannel,
};
use embassy_time::Duration;
use heapless::String;

use crate::SETTINGS_DATA_WATCH;

pub static SPOOLMAN_PIPE: Pipe<CriticalSectionRawMutex, 4096> = Pipe::new();

#[derive(Debug, Clone, PartialEq)]
pub enum SpoolmanCommands {
    Done,
    Get,
    /// (filament_id, td_value)
    Set(u32, Option<f32>, Option<String<6>>),
    SetResponse(u16),
}

pub static SPOOLMAN_CHANNEL: PubSubChannel<CriticalSectionRawMutex, SpoolmanCommands, 2, 2, 2> =
    PubSubChannel::new();
#[embassy_executor::task]
pub async fn spoolman_task(stack: Stack<'static>) {
    let publisher = SPOOLMAN_CHANNEL.publisher().unwrap();
    let mut chan_recv = SPOOLMAN_CHANNEL.subscriber().unwrap();
    loop {
        let command = chan_recv.next_message_pure().await;
        if matches!(
            command,
            SpoolmanCommands::Done | SpoolmanCommands::SetResponse(_)
        ) {
            continue;
        }

        let mut rx_buffer = [0; 4096];
        let mut tx_buffer = [0; 1024];

        let settings = SETTINGS_DATA_WATCH.anon_receiver().try_get().unwrap();
        let host = settings.spoolman_host.unwrap();
        let port = settings.spoolman_port.unwrap();
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));
        socket
            .connect((Ipv4Address::from_str(host.as_str()).unwrap(), port))
            .await
            .unwrap();
        match command {
            SpoolmanCommands::Get => {
                get_filaments(&mut socket, host.as_str(), port).await;
                publisher.publish(SpoolmanCommands::Done).await;
            }

            SpoolmanCommands::Set(id, td, color) => {
                let status = set_filament(&mut socket, id, td, color, host.as_str(), port).await;
                publisher
                    .publish(SpoolmanCommands::SetResponse(status))
                    .await;
            }

            _ => continue,
        }
    }
}

async fn get_filaments<'a>(socket: &mut TcpSocket<'a>, host: &str, port: u16) -> u16 {
    let mut request: String<256> = String::new();
    write!(
        &mut request,
        "GET /api/v1/filament HTTP/1.1\r\n\
  Host: {}:{}\r\n\
  Connection: close\r\n\
  \r\n",
        host, port
    )
    .unwrap();

    socket.write(request.as_bytes()).await.unwrap();

    if extract_headers(socket).await.is_err() {
        return 0;
    };

    let mut buffer = [0u8; 4096];

    loop {
        let n = socket.read(&mut buffer).await.unwrap();
        if n == 0 {
            break;
        }
        SPOOLMAN_PIPE.write_all(&buffer[..n]).await;
    }
    return 0;
}

pub async fn extract_headers<'a>(socket: &mut TcpSocket<'a>) -> Result<(), ()> {
    let mut header_buffer = [0u8; 2048];
    let mut header_len = 0usize;

    loop {
        let n = socket.read(&mut header_buffer[header_len..]).await.unwrap();

        if n == 0 {
            // Connection closed before we found the headers.
            return Err(());
        }

        header_len += n;

        if let Some(pos) = find_header_end(&header_buffer[..header_len]) {
            let body_start = pos + 4;

            // -------------------------------------------------------------
            // Everything after "\r\n\r\n" is already body data.
            // -------------------------------------------------------------

            if body_start < header_len {
                SPOOLMAN_PIPE
                    .write_all(&header_buffer[body_start..header_len])
                    .await;
            }

            break;
        }

        // Don't allow arbitrarily large headers.
        if header_len == header_buffer.len() {
            return Err(());
        }
    }
    return Ok(());
}

async fn set_filament<'a>(
    socket: &mut TcpSocket<'a>,
    filament_id: u32,
    td_value: Option<f32>,
    color: Option<String<6>>,
    host: &str,
    port: u16,
) -> u16 {
    let mut body: String<128> = String::new();
    body.push('{').unwrap();
    let mut first = true;
    if let Some(td) = td_value {
        write!(&mut body, r#""extra":{{"td":"{}"}}"#, td).unwrap();
        first = false;
    }
    if let Some(color) = color {
        if !first {
            body.push(',').unwrap();
        }
        write!(&mut body, r#""color_hex":"{}""#, color).unwrap();
    }
    body.push('}').unwrap();
    let mut request: String<256> = String::new();

    write!(
        &mut request,
        "PATCH /api/v1/filament/{} HTTP/1.1\r\n\
         Host: {}:{}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        filament_id,
        host,
        port,
        body.len(),
        body
    )
    .unwrap();

    if socket.write(request.as_bytes()).await.is_err() {
        return 0;
    }

    read_status_code(socket).await.unwrap_or(0)
}

async fn read_status_code<'a>(socket: &mut TcpSocket<'a>) -> Result<u16, ()> {
    let mut buffer = [0u8; 256];
    let mut len = 0;

    loop {
        if len == buffer.len() {
            return Err(());
        }

        let n = socket.read(&mut buffer[len..]).await.map_err(|_| ())?;

        if n == 0 {
            return Err(());
        }

        len += n;

        if let Some(end) = find_header_end(&buffer[..len]) {
            let headers = &buffer[..end];

            let line_end = headers.windows(2).position(|w| w == b"\r\n").ok_or(())?;

            let status_line = &headers[..line_end];

            // "HTTP/1.1 200 OK"
            if status_line.len() < 12 {
                return Err(());
            }

            let status = core::str::from_utf8(&status_line[9..12])
                .map_err(|_| ())?
                .parse::<u16>()
                .map_err(|_| ())?;

            return Ok(status);
        }
    }
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|window| window == b"\r\n\r\n")
}
