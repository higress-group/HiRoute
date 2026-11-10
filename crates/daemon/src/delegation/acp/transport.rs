//! Bound line allocation before SDK parsing. SDK ByteStreams itself has unbounded lines.
use agent_client_protocol::{Client, ConnectTo, Lines};
use futures::StreamExt;
use std::io;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

use super::ProgressSink;

const MAX_ACP_INBOUND_FRAME_BYTES: usize = 30 * 1024 * 1024;
const MAX_ACP_OUTBOUND_FRAME_BYTES: usize = 1024 * 1024;

pub(super) fn bounded_transport<R, W>(
    reader: R,
    writer: W,
    progress: Option<ProgressSink>,
) -> impl ConnectTo<Client>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let incoming = FramedRead::new(
        reader,
        LinesCodec::new_with_max_length(MAX_ACP_INBOUND_FRAME_BYTES),
    )
    .map(move |line| {
        if matches!(line, Err(LinesCodecError::MaxLineLengthExceeded))
            && let Some(progress) = &progress
        {
            progress.push(&format!(
                "[HiRoute] ACP_FRAME_TOO_LARGE: The incoming ACP frame exceeded the {MAX_ACP_INBOUND_FRAME_BYTES}-byte limit.\n"
            ));
        }
        line.map_err(io::Error::other)
    });
    let outgoing = futures::sink::unfold(writer, |mut writer, line: String| async move {
        if line.len() > MAX_ACP_OUTBOUND_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ACP frame too large",
            ));
        }
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok::<_, io::Error>(writer)
    });
    Lines::new(Box::pin(outgoing), incoming)
}
