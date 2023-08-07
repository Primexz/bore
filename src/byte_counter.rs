//! A module to count bytes read and written in tcp streams.

use std::io::Result as IoResult;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::debug;

use crate::metrics;

/// Wrapper around a streams that counts bytes read and written.
pub struct CountingStream<S> {
    inner: S,
}

impl<S> CountingStream<S> {
    /// Create a new counting stream.
    pub fn new(inner: S) -> Self {
        CountingStream { inner }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for CountingStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<IoResult<()>> {
        let this = &mut *self;

        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(_)) => {
                metrics::INCOMING_BYTES.inc_by(buf.filled().len() as u64);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CountingStream<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<IoResult<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                metrics::OUTGOING_BYTES.inc_by(n as u64);
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<IoResult<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context) -> Poll<IoResult<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Function to calculate bytes in and out per second.
pub fn bytes_per_second_calculator() {
    tokio::spawn(async move {
        let mut old_bytes_out = 0;
        let mut old_bytes_in = 0;

        loop {
            let new_bytes_out = metrics::OUTGOING_BYTES.get();
            let new_bytes_in = metrics::INCOMING_BYTES.get();

            let bytes_per_second_out = new_bytes_out - old_bytes_out;
            let bytes_per_second_in = new_bytes_in - old_bytes_in;

            metrics::OUTGOING_BYTES_PER_SECOND.set(bytes_per_second_out as i64);
            metrics::INCOMING_BYTES_PER_SECOND.set(bytes_per_second_in as i64);

            debug!("{}", "-".repeat(25));
            debug!("bytes per second out: {}", bytes_per_second_out);
            debug!("bytes per second in: {}", bytes_per_second_in);
            debug!("{}", "-".repeat(25));

            old_bytes_out = new_bytes_out;
            old_bytes_in = new_bytes_in;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}
