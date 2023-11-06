use hyper::server::accept::Accept;
use log::debug;
use std::fs;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::net::UnixListener;

pub struct UnixDomainSocket {
    inner: UnixListener,
}

impl UnixDomainSocket {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let listener = UnixListener::bind(path)?;
        Ok(Self { inner: listener })
    }
}

impl Drop for UnixDomainSocket {
    fn drop(&mut self) {
        let addr = self
            .inner
            .local_addr()
            .expect("failed to get local address from listener");
        let path = addr
            .as_pathname()
            .expect("failed to get path name from local address");
        debug!("Removing socket file {path}", path = path.display());
        fs::remove_file(path).ok();
    }
}

impl Accept for UnixDomainSocket {
    type Conn = tokio::net::UnixStream;
    type Error = std::io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        match self.inner.poll_accept(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok((socket, _addr))) => Poll::Ready(Some(Ok(socket))),
            Poll::Ready(Err(err)) => Poll::Ready(Some(Err(err))),
        }
    }
}
