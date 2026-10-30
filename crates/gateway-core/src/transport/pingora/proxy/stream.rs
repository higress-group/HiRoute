//! Preserve Pingora's stream metadata across a TLS layer over a boxed stream.
use super::super::*;

pub(super) struct LayeredStream(pub Stream);

impl std::fmt::Debug for LayeredStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LayeredStream")
    }
}
impl AsyncRead for LayeredStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.0.as_mut()).poll_read(cx, buf)
    }
}
impl AsyncWrite for LayeredStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.0.as_mut()).poll_write(cx, buf)
    }
    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(self.0.as_mut()).poll_write_vectored(cx, bufs)
    }
    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.0.as_mut()).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.0.as_mut()).poll_shutdown(cx)
    }
}
#[async_trait]
impl Shutdown for LayeredStream {
    async fn shutdown(&mut self) {
        Shutdown::shutdown(self.0.as_mut()).await
    }
}
impl UniqueID for LayeredStream {
    fn id(&self) -> UniqueIDType {
        self.0.id()
    }
}
impl Ssl for LayeredStream {
    fn get_ssl(&self) -> Option<&pingora_core::protocols::tls::TlsRef> {
        self.0.get_ssl()
    }
    fn get_ssl_digest(&self) -> Option<Arc<pingora_core::protocols::tls::SslDigest>> {
        self.0.get_ssl_digest()
    }
    fn selected_alpn_proto(&self) -> Option<ALPN> {
        self.0.selected_alpn_proto()
    }
}
impl GetTimingDigest for LayeredStream {
    fn get_timing_digest(&self) -> Vec<Option<pingora_core::protocols::TimingDigest>> {
        self.0.get_timing_digest()
    }
}
impl GetProxyDigest for LayeredStream {
    fn get_proxy_digest(&self) -> Option<Arc<pingora_core::protocols::raw_connect::ProxyDigest>> {
        self.0.get_proxy_digest()
    }
}
impl GetSocketDigest for LayeredStream {
    fn get_socket_digest(&self) -> Option<Arc<pingora_core::protocols::SocketDigest>> {
        self.0.get_socket_digest()
    }
}
#[async_trait]
impl Peek for LayeredStream {
    async fn try_peek(&mut self, buf: &mut [u8]) -> io::Result<bool> {
        self.0.try_peek(buf).await
    }
}
