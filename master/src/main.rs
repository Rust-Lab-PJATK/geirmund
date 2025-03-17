use futures::{Stream, StreamExt};
use proto::{MasterPacket, WorkerPacket};
use std::{net::ToSocketAddrs, pin::Pin};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{transport::Server, Code, Request, Response, Status, Streaming};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub struct MasterServer {
    cancellation_token: CancellationToken,
    send_response_tx: broadcast::Sender<MasterPacket>,
    send_response_rx: broadcast::Receiver<MasterPacket>,
    receive_request_tx: broadcast::Sender<(core::net::SocketAddr, WorkerPacket)>,
}

#[tonic::async_trait]
impl proto::master_server::Master for MasterServer {
    // this name is awful, but it's required by tonic.
    // I did not have any choice.
    type StreamStream = Pin<Box<dyn Stream<Item = Result<MasterPacket, Status>> + Send>>;

    async fn stream(
        &self,
        request: Request<Streaming<WorkerPacket>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        let socket_addr = request.remote_addr().ok_or_else(|| {
            tracing::error!("failed to acquire worker remote address, disconnecting the worker");
            return Status::new(Code::Internal, "failed to acquire worker remote address");
        })?;

        tracing::info!("new worker connection from address: {socket_addr}");

        // Receive request from worker

        let mut in_stream = request.into_inner();

        let receive_request_tx = self.receive_request_tx.clone();
        let receive_request_socket_addr = socket_addr.clone();
        let receive_request_cancellation_token = self.cancellation_token.clone();
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    Some(received_event) = in_stream.next() => match received_event {
                        Ok(event) => event,
                        Err(e) => {
                            tracing::error!("GRPC Error received from worker on address {receive_request_socket_addr:?}: {e}");
                            continue;
                        }
                    },
                    _ = receive_request_cancellation_token.cancelled() => break,
                };

                receive_request_tx
                    .send((receive_request_socket_addr, event))
                    .unwrap();
            }
        });

        // Send response to worker

        let mut send_response_tx = self.send_response_tx.subscribe();

        let (out_stream_tx, out_stream_rx) = mpsc::channel(128);

        let send_response_socket_addr = socket_addr;
        let send_response_cancellation_token = self.cancellation_token.clone();
        tokio::spawn(async move {
            loop {
                let event = tokio::select! {
                    received_event = send_response_tx.recv() => match received_event {
                        Ok(event) => event,
                        Err(e) => {
                            tracing::error!("recoverable error received on local_send_request_receiver on worker address {send_response_socket_addr:?}: {e}");
                            continue;
                        }
                    },
                    _ = send_response_cancellation_token.cancelled() => break,
                };

                if let Err(e) = out_stream_tx.send(Ok(event)).await {
                    tracing::error!("recoverable error received when trying to pass MasterPacket from local_send_request_receiver to tx: {e}");
                }
            }
        });

        let out_stream = ReceiverStream::new(out_stream_rx);

        Ok(Response::new(Box::pin(out_stream) as Self::StreamStream))
    }
}

fn setup_logging() {
    let stdout_layer = tracing_subscriber::fmt::Layer::new()
        .compact()
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .with_writer(std::io::stdout);

    let file_layer = tracing_subscriber::fmt::Layer::new()
        .compact()
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .with_writer(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("master.log")
                .unwrap(),
        );

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .init();
}

#[tokio::main]
async fn main() {
    setup_logging();

    tracing::info!("Creating reflection service...");

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    let cancellation_token = CancellationToken::new();

    let (send_response_tx, send_response_rx) = broadcast::channel(128);
    let (receive_request_tx, receive_request_rx) = broadcast::channel(128);

    let server = MasterServer {
        cancellation_token: cancellation_token.clone(),
        send_response_tx,
        send_response_rx,
        receive_request_tx,
    };

    tracing::info!("Starting GRPC tcp listener... (there will be no confirmation log)");

    Server::builder()
        .add_service(reflection_service)
        .add_service(proto::master_server::MasterServer::new(server))
        .serve("[::1]:50051".to_socket_addrs().unwrap().next().unwrap())
        .await
        .unwrap();
}
