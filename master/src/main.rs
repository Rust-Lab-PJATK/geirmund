use futures::Stream;
use proto::{MasterPacket, WorkerPacket};
use std::{net::ToSocketAddrs, pin::Pin};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{transport::Server, Request, Response, Status, Streaming};
use tracing::Level;

pub struct MasterServer {
    cancellation_token: CancellationToken,
    send_request_receiver: mpsc::Receiver<MasterPacket>,
    receive_response_sender: mpsc::Sender<WorkerPacket>,
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
        let in_stream = request.into_inner();

        let (tx, rx) = mpsc::channel(128);

        let cancellation_token = self.cancellation_token.clone();
        tokio::spawn(async move { while !cancellation_token.is_cancelled() {} });

        let out_stream = ReceiverStream::new(rx);

        Ok(Response::new(Box::pin(out_stream) as Self::StreamStream))
    }
}

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "debug");

    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .with_max_level(Level::DEBUG) // TODO by env
        .with_writer(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("master.log")
                .unwrap(),
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber).unwrap();

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    let cancellation_token = CancellationToken::new();

    let (send_request_sender, send_request_receiver) = mpsc::channel(128);
    let (receive_response_sender, receive_response_receiver) = mpsc::channel(128);

    let server = MasterServer {
        cancellation_token: cancellation_token.clone(),
        send_request_receiver,
        receive_response_sender,
    };

    Server::builder()
        .add_service(reflection_service)
        .add_service(proto::master_server::MasterServer::new(server))
        .serve("[::1]:50051".to_socket_addrs().unwrap().next().unwrap())
        .await
        .unwrap();
}
