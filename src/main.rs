mod certs;
mod hybrid;

#[cfg(all(unix, feature = "unix-domain-sockets"))]
mod sockets;

use crate::hybrid::HybridMakeService;
#[cfg(all(unix, feature = "unix-domain-sockets"))]
use crate::sockets::UnixDomainSocket;
use axum::routing::IntoMakeService;
use axum::Router;
use futures_util::StreamExt;
use hyper::server::conn::AddrIncoming;
use hyper::Server;
use log::{error, info, warn};
use std::future::Future;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::str::FromStr;
use tls_listener::TlsListener;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Sender;
use tokio::task::JoinSet;
use tonic::transport::server::Routes;

mod proto {
    tonic::include_proto!("example");

    pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
        tonic::include_file_descriptor_set!("example_descriptor");
}

#[derive(Default)]
pub struct MyGrpcService {}

#[tonic::async_trait]
impl proto::your_service_server::YourService for MyGrpcService {
    async fn your_method(
        &self,
        request: tonic::Request<proto::YourRequest>,
    ) -> Result<tonic::Response<proto::YourResponse>, tonic::Status> {
        info!("Handling gRPC request from {:?}", request.remote_addr());

        let reply = proto::YourResponse {
            reply: format!("Hello {}!", request.into_inner().message),
        };
        Ok(tonic::Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // Load .env files.
    dotenvy::dotenv().ok();

    // Set up logging.
    initialize_logging(LoggingStyle::Json);

    // Provide a signal that can be used to shut down the server.
    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    register_shutdown_handler(shutdown_tx.clone());

    // Build the web and gRPC service.
    let grpc_service = build_grpc_service();
    let axum_make_svc = build_web_service();

    // Combine web and gRPC into a hybrid service.
    let service = HybridMakeService::new(axum_make_svc, grpc_service);

    // Combine the server futures.
    let mut futures = JoinSet::new();

    // Bind first hyper HTTP server.
    let socket_addr =
        SocketAddr::from_str("127.0.0.1:36849").expect("failed to parse socket address");
    let server_a = create_hyper_server(service.clone(), socket_addr, &shutdown_tx);
    futures.spawn(server_a);

    // Bind second hyper HTTP server.
    let socket_addr =
        SocketAddr::from_str("127.1.0.1:36849").expect("failed to parse socket address");
    let server_b = create_hyper_server(service.clone(), socket_addr, &shutdown_tx);
    futures.spawn(server_b);

    // Bind third hyper HTTP server (using TLS).
    let socket_addr =
        SocketAddr::from_str("127.0.0.1:36850").expect("failed to parse socket address");
    let server_c = create_hyper_server_tls(service.clone(), socket_addr, &shutdown_tx);
    futures.spawn(server_c);

    // Bind fourth server to Unix Domain Socket.
    #[cfg(all(unix, feature = "unix-domain-sockets"))]
    {
        let socket_addr = std::path::PathBuf::from("/tmp/cohosting.sock");
        let server_d = create_hyper_server_uds(service, socket_addr, &shutdown_tx);
        futures.spawn(server_d);
    }

    // Wait for all servers to stop.
    info!("Starting servers");
    while let Some(join_result) = futures.join_next().await {
        match join_result {
            Ok(result) => match result {
                Ok(()) => {
                    info!("A Hyper server stopped gracefully");
                }
                Err(e) => {
                    error!("A Hyper server terminated with an error: {}", e);
                }
            },
            Err(e) => {
                error!("An error occurred while joining the server results: {}", e);
            }
        }

        // Ensure that all other servers also shut down in presence
        // of an error of any one of them.
        shutdown_tx.send(()).ok();
    }

    info!("Shutting down application");
    ExitCode::SUCCESS
}

fn build_web_service() -> IntoMakeService<Router> {
    let app = Router::new().route("/", axum::routing::get(root_handler));
    app.into_make_service()
}

fn build_grpc_service() -> Routes {
    let grpc_service = MyGrpcService::default();

    // Build the gRPC reflections service
    let grpc_reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build()
        .unwrap();

    tonic::transport::Server::builder()
        .add_service(grpc_reflection_service)
        .add_service(proto::your_service_server::YourServiceServer::new(
            grpc_service,
        ))
        .into_service()
}

fn create_hyper_server(
    service: HybridMakeService<IntoMakeService<Router>, Routes>,
    socket_addr: SocketAddr,
    shutdown_tx: &Sender<()>,
) -> impl Future<Output = Result<(), hyper::Error>> + Send + use<> {
    info!("Binding server to {}", socket_addr);
    Server::try_bind(&socket_addr)
        .map_err(|e| {
            error!(
                "Unable to bind to {addr}: {error}",
                addr = socket_addr,
                error = e
            );
            // No servers are currently running since no await was called on any
            // of them yet. Therefore, exiting here is "graceful".
            ExitCode::from(exitcode::NOPERM as u8)
        })
        .expect("failed to bind Hyper server") // TODO: Actually return error
        .serve(service)
        .with_graceful_shutdown({
            let mut shutdown_rx = shutdown_tx.subscribe();
            async move {
                shutdown_rx.recv().await.ok();
                info!("Graceful shutdown initiated on Hyper server")
            }
        })
}

#[cfg(all(unix, feature = "unix-domain-sockets"))]
fn create_hyper_server_uds(
    service: HybridMakeService<IntoMakeService<Router>, Routes>,
    socket_path: std::path::PathBuf,
    shutdown_tx: &Sender<()>,
) -> impl Future<Output = Result<(), hyper::Error>> + Send + use<> {
    info!("Binding server to {}", socket_path.display());

    let incoming = match UnixDomainSocket::new(&socket_path) {
        Ok(listener) => listener,
        Err(e) => {
            error!(
                "Failed to bind to socket {addr}: {error}",
                addr = socket_path.display(),
                error = e
            );
            panic!("{}", e);
        }
    };
    Server::builder(incoming)
        .serve(service)
        .with_graceful_shutdown({
            let mut shutdown_rx = shutdown_tx.subscribe();
            async move {
                shutdown_rx.recv().await.ok();
                info!("Graceful shutdown initiated on Hyper server")
            }
        })
}

fn create_hyper_server_tls(
    service: HybridMakeService<IntoMakeService<Router>, Routes>,
    socket_addr: SocketAddr,
    shutdown_tx: &Sender<()>,
) -> impl Future<Output = Result<(), hyper::Error>> + Send + use<> {
    info!("Binding server to {}", socket_addr);
    let listener = AddrIncoming::bind(&socket_addr)
        .map_err(|e| {
            error!(
                "Unable to bind to {addr}: {error}",
                addr = socket_addr,
                error = e
            );
            // No servers are currently running since no await was called on any
            // of them yet. Therefore, exiting here is "graceful".
            ExitCode::from(exitcode::NOPERM as u8)
        })
        .expect("failed to bind Hyper server"); // TODO: Actually return error

    // Create a TLS listener and filter out all invalid connections.
    let incoming = TlsListener::new(certs::tls_acceptor(), listener)
        .connections()
        .filter(handle_tls_connect_error);

    Server::builder(hyper::server::accept::from_stream(incoming))
        .serve(service)
        .with_graceful_shutdown({
            let mut shutdown_rx = shutdown_tx.subscribe();
            async move {
                shutdown_rx.recv().await.ok();
                info!("Graceful shutdown initiated on first Hyper server")
            }
        })
}

async fn root_handler() -> String {
    info!("Handling HTTP request");
    String::from("ok")
}

fn register_shutdown_handler(shutdown_tx: broadcast::Sender<()>) {
    ctrlc::set_handler(move || {
        warn!("Caught SIGINT from OS");
        shutdown_tx.send(()).ok();
    })
    .expect("Error setting process termination handler");
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum LoggingStyle {
    /// Uses compact logging.
    Compact,
    /// Uses JSON formatted logging
    Json,
}

pub fn initialize_logging<Style>(style: Style)
where
    Style: std::borrow::Borrow<LoggingStyle>,
{
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing::metadata::LevelFilter::INFO.into())
        .from_env_lossy();

    let formatter = tracing_subscriber::fmt()
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(true)
        .with_target(true)
        .with_env_filter(filter);

    match style.borrow() {
        LoggingStyle::Compact => formatter.init(),
        LoggingStyle::Json => formatter.json().init(),
    }
}

fn handle_tls_connect_error<AddrStream>(
    result: &Result<
        tokio_rustls::server::TlsStream<AddrStream>,
        tls_listener::Error<std::io::Error, std::io::Error, SocketAddr>,
    >,
) -> std::future::Ready<bool> {
    if let Err(err) = result {
        error!("Error: {:?}", err);
        std::future::ready(false)
    } else {
        std::future::ready(true)
    }
}
