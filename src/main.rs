use axum::Router;
use futures::TryFutureExt;
use hyper::Server;
use log::{error, info, warn};
use std::net::SocketAddr;
use std::process::ExitCode;
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tower::ServiceBuilder;

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

    // Build the gRPC service.
    let grpc_service = MyGrpcService::default();

    // Build the gRPC reflections service
    let grpc_reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build()
        .unwrap();

    let grpc_addr = "127.0.0.1:50052".parse().unwrap();
    let grpc = tonic::transport::Server::builder()
        .add_service(grpc_reflection_service)
        .add_service(proto::your_service_server::YourServiceServer::new(
            grpc_service,
        ))
        .serve(grpc_addr)
        .map_err(|e| ServerError::TonicError(e));

    // Build an Axum router.
    let app = Router::new().route("/", axum::routing::get(root_handler));

    // Convert into a Tower service.
    let make_svc = app.into_make_service();
    let service_builder = ServiceBuilder::new().service(make_svc);

    // Bind first hyper HTTP server.
    let socket_addr =
        SocketAddr::from_str("127.0.0.1:36849").expect("failed to parse socket address");
    let server_a = Server::try_bind(&socket_addr)
        .map_err(|e| {
            error!(
                "Unable to bind to {addr}: {error}",
                addr = socket_addr,
                error = e
            );
            // No servers are currently running since no await was called on any
            // of them yet. Therefore, exiting here is "graceful".
            return ExitCode::from(exitcode::NOPERM as u8);
        })
        .expect("failed to bind first Hyper server")
        .serve(service_builder.clone())
        .with_graceful_shutdown({
            let mut shutdown_rx = shutdown_tx.subscribe();
            async move {
                shutdown_rx.recv().await.ok();
                info!("Graceful shutdown initiated on first Hyper server")
            }
        })
        .map_err(|e| ServerError::HyperError(e));

    // Bind second hyper HTTP server.
    let socket_addr =
        SocketAddr::from_str("127.1.0.1:36849").expect("failed to parse socket address");
    let server_b = Server::try_bind(&socket_addr)
        .map_err(|e| {
            error!(
                "Unable to bind to {addr}: {error}",
                addr = socket_addr,
                error = e
            );
            // No servers are currently running since no await was called on any
            // of them yet. Therefore, exiting here is "graceful".
            return ExitCode::from(exitcode::NOPERM as u8);
        })
        .expect("failed to bind second Hyper server")
        .serve(service_builder.clone())
        .with_graceful_shutdown({
            let mut shutdown_rx = shutdown_tx.subscribe();
            async move {
                shutdown_rx.recv().await.ok();
                info!("Graceful shutdown initiated on second Hyper server")
            }
        })
        .map_err(|e| ServerError::HyperError(e));

    // Combine the server futures.
    let mut futures = JoinSet::new();
    futures.spawn(server_a);
    futures.spawn(server_b);
    futures.spawn(grpc);

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

#[derive(Debug, thiserror::Error)]
enum ServerError {
    #[error(transparent)]
    HyperError(#[from] hyper::Error),
    #[error(transparent)]
    TonicError(#[from] tonic::transport::Error),
}
