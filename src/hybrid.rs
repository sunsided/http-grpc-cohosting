use axum::http::Response;
use hyper::body::HttpBody;
use hyper::service::Service;
use hyper::{Body, HeaderMap, Request, Version};
use pin_project::pin_project;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

#[derive(Clone)]
pub struct HybridMakeService<Web, Grpc> {
    make_web: Web,
    grpc: Grpc,
}

impl<Web, Grpc> HybridMakeService<Web, Grpc> {
    pub const fn new(make_web: Web, grpc: Grpc) -> Self {
        HybridMakeService { make_web, grpc }
    }
}

/// A Tower [`Service`] implementing the factory of a [`HybridService`].
impl<ConnInfo, MakeWeb, Grpc> Service<ConnInfo> for HybridMakeService<MakeWeb, Grpc>
where
    MakeWeb: Service<ConnInfo>,
    Grpc: Clone,
{
    /// Responses given by the service.
    type Response = HybridService<MakeWeb::Response, Grpc>;

    /// Errors produced by the service.
    type Error = MakeWeb::Error;

    /// The future response value.
    type Future = HybridMakeServiceFuture<MakeWeb::Future, Grpc>;

    /// Returns `Poll::Ready(Ok(()))` when the service is able to process requests.
    ///
    /// If the service is at capacity, then `Poll::Pending` is returned and the task
    /// is notified when the service becomes ready again. This function is
    /// expected to be called while on a task. Generally, this can be done with
    /// a simple `futures::future::poll_fn` call.
    ///
    /// If `Poll::Ready(Err(_))` is returned, the service is no longer able to service requests
    /// and the caller should discard the service instance.
    ///
    /// Once `poll_ready` returns `Poll::Ready(Ok(()))`, a request may be dispatched to the
    /// service using `call`. Until a request is dispatched, repeated calls to
    /// `poll_ready` must return either `Poll::Ready(Ok(()))` or `Poll::Ready(Err(_))`.
    ///
    /// Note that `poll_ready` may reserve shared resources that are consumed in a subsequent
    /// invocation of `call`. Thus, it is critical for implementations to not assume that `call`
    /// will always be invoked and to ensure that such resources are released if the service is
    /// dropped before `call` is invoked or the future returned by `call` is dropped before it
    /// is polled.
    fn poll_ready(&mut self, cx: &mut std::task::Context) -> Poll<Result<(), Self::Error>> {
        self.make_web.poll_ready(cx)
    }

    fn call(&mut self, conn_info: ConnInfo) -> Self::Future {
        HybridMakeServiceFuture {
            web_future: self.make_web.call(conn_info),
            grpc: Some(self.grpc.clone()),
        }
    }
}

/// The future returned by [`HybridMakeService`]. Returns a [`HybridService`].
#[pin_project]
pub struct HybridMakeServiceFuture<WebFuture, Grpc> {
    #[pin]
    web_future: WebFuture,
    grpc: Option<Grpc>,
}

impl<WebFuture, Web, WebError, Grpc> Future for HybridMakeServiceFuture<WebFuture, Grpc>
where
    WebFuture: Future<Output = Result<Web, WebError>>,
{
    type Output = Result<HybridService<Web, Grpc>, WebError>;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context) -> Poll<Self::Output> {
        let this = self.project();
        match this.web_future.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(web)) => Poll::Ready(Ok(HybridService {
                web,
                grpc: this.grpc.take().expect("Cannot poll twice!"),
            })),
        }
    }
}

/// Handles a request and returns a [`HybridFuture`] returning a [`HybridBody`].
///
/// This service switches between gRPC and regular HTTP traffic and forwards the request
/// to either the `Web` (e.g. Axum) or `Grpc` (e.g. Tonic) [`Service`].
pub struct HybridService<Web, Grpc> {
    web: Web,
    grpc: Grpc,
}

impl<Web, Grpc, WebBody, GrpcBody> Service<Request<Body>> for HybridService<Web, Grpc>
where
    Web: Service<Request<Body>, Response = Response<WebBody>>,
    Grpc: Service<Request<Body>, Response = Response<GrpcBody>>,
    Web::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
    Grpc::Error: Into<Box<dyn Error + Send + Sync + 'static>>,
{
    /// Responses given by the service.
    type Response = Response<HybridBody<WebBody, GrpcBody>>;

    /// Errors produced by the service.
    type Error = Box<dyn Error + Send + Sync + 'static>;

    /// The future response value.
    type Future = HybridFuture<Web::Future, Grpc::Future>;

    /// Returns `Poll::Ready(Ok(()))` when the service is able to process requests.
    ///
    /// If the service is at capacity, then `Poll::Pending` is returned and the task
    /// is notified when the service becomes ready again. This function is
    /// expected to be called while on a task. Generally, this can be done with
    /// a simple `futures::future::poll_fn` call.
    ///
    /// If `Poll::Ready(Err(_))` is returned, the service is no longer able to service requests
    /// and the caller should discard the service instance.
    ///
    /// Once `poll_ready` returns `Poll::Ready(Ok(()))`, a request may be dispatched to the
    /// service using `call`. Until a request is dispatched, repeated calls to
    /// `poll_ready` must return either `Poll::Ready(Ok(()))` or `Poll::Ready(Err(_))`.
    ///
    /// Note that `poll_ready` may reserve shared resources that are consumed in a subsequent
    /// invocation of `call`. Thus, it is critical for implementations to not assume that `call`
    /// will always be invoked and to ensure that such resources are released if the service is
    /// dropped before `call` is invoked or the future returned by `call` is dropped before it
    /// is polled.
    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.web.poll_ready(cx) {
            Poll::Ready(Ok(())) => match self.grpc.poll_ready(cx) {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
                Poll::Pending => Poll::Pending,
            },
            Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
            Poll::Pending => Poll::Pending,
        }
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        if is_grpc_request(&req) {
            HybridFuture::Grpc(self.grpc.call(req))
        } else {
            HybridFuture::Web(self.web.call(req))
        }
    }
}

fn is_grpc_request<Body>(req: &Request<Body>) -> bool {
    // gRPC uses HTTP/2 frames.
    if req.version() != Version::HTTP_2 {
        return false;
    }

    // The content-type header needs to start with `application/grpc`
    const EXPECTED: &[u8] = b"application/grpc";
    if let Some(content_type) = req
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .map(|x| x.as_bytes())
    {
        content_type.len() >= EXPECTED.len() && content_type[..EXPECTED.len()] == *EXPECTED
    } else {
        false
    }
}

#[pin_project(project = HybridBodyProj)]
pub enum HybridBody<WebBody, GrpcBody> {
    Web(#[pin] WebBody),
    Grpc(#[pin] GrpcBody),
}

impl<WebBody, GrpcBody> HttpBody for HybridBody<WebBody, GrpcBody>
where
    WebBody: HttpBody + Send + Unpin,
    GrpcBody: HttpBody<Data = WebBody::Data> + Send + Unpin,
    WebBody::Error: Error + Send + Sync + 'static,
    GrpcBody::Error: Error + Send + Sync + 'static,
{
    type Data = WebBody::Data;
    type Error = Box<dyn Error + Send + Sync + 'static>;

    fn poll_data(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context,
    ) -> Poll<Option<Result<Self::Data, Self::Error>>> {
        match self.project() {
            HybridBodyProj::Web(b) => b.poll_data(cx).map_err(|e| e.into()),
            HybridBodyProj::Grpc(b) => b.poll_data(cx).map_err(|e| e.into()),
        }
    }

    fn poll_trailers(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context,
    ) -> Poll<Result<Option<HeaderMap>, Self::Error>> {
        match self.project() {
            HybridBodyProj::Web(b) => b.poll_trailers(cx).map_err(|e| e.into()),
            HybridBodyProj::Grpc(b) => b.poll_trailers(cx).map_err(|e| e.into()),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            HybridBody::Web(b) => b.is_end_stream(),
            HybridBody::Grpc(b) => b.is_end_stream(),
        }
    }
}

/// Future returned by [`HybridService`].
#[pin_project(project = HybridFutureProj)]
pub enum HybridFuture<WebFuture, GrpcFuture> {
    Web(#[pin] WebFuture),
    Grpc(#[pin] GrpcFuture),
}

impl<WebFuture, GrpcFuture, WebBody, GrpcBody, WebError, GrpcError> Future
    for HybridFuture<WebFuture, GrpcFuture>
where
    WebFuture: Future<Output = Result<Response<WebBody>, WebError>>,
    GrpcFuture: Future<Output = Result<Response<GrpcBody>, GrpcError>>,
    WebError: Into<Box<dyn Error + Send + Sync + 'static>>,
    GrpcError: Into<Box<dyn Error + Send + Sync + 'static>>,
{
    type Output =
        Result<Response<HybridBody<WebBody, GrpcBody>>, Box<dyn Error + Send + Sync + 'static>>;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context) -> Poll<Self::Output> {
        match self.project() {
            HybridFutureProj::Web(a) => match a.poll(cx) {
                Poll::Ready(Ok(res)) => Poll::Ready(Ok(res.map(HybridBody::Web))),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
                Poll::Pending => Poll::Pending,
            },
            HybridFutureProj::Grpc(b) => match b.poll(cx) {
                Poll::Ready(Ok(res)) => Poll::Ready(Ok(res.map(HybridBody::Grpc))),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}
