use picoserve::{
    response::{
        File, IntoResponse,
        chunked::{ChunkWriter, ChunkedResponse, Chunks, ChunksWritten},
    },
    routing::{get, get_service},
};

use crate::tasks::http::{AppState, RedirectToPortal};

pub fn frontend_router()
-> picoserve::Router<impl picoserve::routing::PathRouter<AppState>, AppState> {
    picoserve::Router::from_service(RedirectToPortal)
        .route("/", get_service(get_index()))
        .route("/app.js", get(get_js))
        .route("/app.css", get_service(get_css()))
}

fn get_index() -> File {
    File::with_content_type_and_headers(
        "text/html",
        include_bytes!("frontend/dist/index.html.gz").as_slice(),
        &[("Content-Encoding", "gzip")],
    )
}
async fn get_js() -> impl IntoResponse {
    ChunkedResponse::new(CompressedFile {
        data: include_bytes!("frontend/dist/app.js.gz").as_slice(),
        content_type: "application/javascript",
    })
    .into_response()
    .with_header("Content-Encoding", "gzip")
}

fn get_css() -> File {
    File::with_content_type_and_headers(
        "text/css",
        include_bytes!("frontend/dist/app.css.gz").as_slice(),
        &[("Content-Encoding", "gzip")],
    )
}

struct CompressedFile {
    data: &'static [u8],
    content_type: &'static str,
}

impl Chunks for CompressedFile {
    fn content_type(&self) -> &'static str {
        self.content_type
    }

    async fn write_chunks<W: picoserve::io::Write>(
        self,
        mut chunk_writer: ChunkWriter<W>,
    ) -> Result<ChunksWritten, W::Error> {
        const CHUNK_SIZE: usize = 256; // Send in 512-byte chunks

        for chunk in self.data.chunks(CHUNK_SIZE) {
            chunk_writer.write_chunk(chunk).await?;
        }

        chunk_writer.finalize().await
    }
}
