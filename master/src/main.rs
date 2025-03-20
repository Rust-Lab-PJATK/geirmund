use futures::{Stream, StreamExt};
use proto::{hello_command_response, hello_command_response_error, HelloCommandResponse, HelloCommandResponseError, MasterPacket, WorkerPacket};
use proto::master_packet::Msg as MasterPacketMsg;
use tracing::Level;
use std::{collections::HashMap, net::{SocketAddr, ToSocketAddrs}, pin::Pin};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{transport::Server, Code, Request, Response, Status, Streaming};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod protobuf;

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
                let maybe_event = tokio::select! {
                    maybe_event = in_stream.next() => maybe_event,
                    _ = receive_request_cancellation_token.cancelled() => {
                        break
                    },
                };

                let event = match maybe_event {
                    Some(Ok(event)) => event,
                    Some(Err(e)) => {
                        tracing::error!(
                            "GRPC Error received from worker on address {receive_request_socket_addr:?}: {e}"
                        );
                        continue;
                    }
                    None => break,
                };

                tracing::debug!(event = tracing::field::debug(&event), "Request received");

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

    let cancellation_token = CancellationToken::new();

    let (send_response_tx, send_response_rx) = broadcast::channel(128);
    let (receive_request_tx, receive_request_rx) = broadcast::channel(128);

    let server = MasterServer {
        cancellation_token: cancellation_token.clone(),
        send_response_tx: send_response_tx.clone(),
        send_response_rx,
        receive_request_tx,
    };

    tracing::info!("Starting GRPC tcp listener... (there will be no confirmation log)");

    let grpc_server_fut = start_grpc_listener(cancellation_token.clone(), server).await;

    let respond_on_commands_cancellation_token = cancellation_token.clone();
    let respond_on_commands_fut = start_respond_on_commands_worker(
        respond_on_commands_cancellation_token,
        receive_request_rx,
        send_response_tx,
    );

    let (grpc_server_result, respond_on_commands_result) =
        tokio::join!(grpc_server_fut, respond_on_commands_fut);

    grpc_server_result.unwrap().unwrap().unwrap();
    respond_on_commands_result.unwrap();
}

async fn start_grpc_listener(
    cancellation_token: CancellationToken,
    server: MasterServer,
) -> JoinHandle<Option<Result<(), tonic::transport::Error>>> {
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(proto::FILE_DESCRIPTOR_SET)
        .build_v1()
        .unwrap();

    let (ready_tx, ready_rx) = oneshot::channel();

    let fut = tokio::spawn(async move {
        cancellation_token
            .run_until_cancelled(async {
                ready_tx.send(()).unwrap();

                Server::builder()
                    .add_service(reflection_service)
                    .add_service(proto::master_server::MasterServer::new(server))
                    .serve("0.0.0.0:50051".to_socket_addrs().unwrap().next().unwrap())
                    .await
            })
            .await
    });

    ready_rx.await.unwrap();

    fut
}

fn start_respond_on_commands_worker(
    cancellation_token: CancellationToken,
    receive_request_rx: broadcast::Receiver<(core::net::SocketAddr, WorkerPacket)>,
    send_response_tx: broadcast::Sender<MasterPacket>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        respond_on_commands(cancellation_token, receive_request_rx, send_response_tx).await
    })
}

async fn respond_on_commands(
    cancellation_token: CancellationToken,
    mut receive_request_rx: broadcast::Receiver<(core::net::SocketAddr, WorkerPacket)>,
    send_response_tx: broadcast::Sender<MasterPacket>,
) {
    let mut clients: HashMap<SocketAddr, String> = HashMap::new();

    loop {
        let request = tokio::select! {
            received_req = receive_request_rx.recv() => match received_req {
                Ok(received_req) => received_req,
                Err(e) => {
                    tracing::error!(error = tracing::field::debug(e), "Recoverable error received while listening for new requests");
                    continue;
                },
            },
            _ = cancellation_token.cancelled() => {break;}
        };

        let (socket_addr, _) = request;

        match request.1 {
            WorkerPacket {
                msg: Some(proto::worker_packet::Msg::HelloCommand(hello_command)),
            } => {
                tracing::info!(socket_addr = ?socket_addr, requested_worker_name = %hello_command.name, "Received Hello! from worker");

                if let Some(current_name) = clients.get(&socket_addr) {
                    send_response_tx.send(
                        protobuf::master::HelloCommandResponse::you_already_have_a_name_error(current_name.clone())
                    )
                        .unwrap();

                    tracing::error!(socket_addr = ?socket_addr, current_name = %current_name, requested_worker_name = %hello_command.name, "Worker requested to be registered as a new worker, but he is already registered");
                } else if clients.values().into_iter().find(|addr| **addr == hello_command.name).is_some() {
                    send_response_tx.send(
                        protobuf::master::HelloCommandResponse::worker_with_given_name_already_exists(hello_command.name.clone())
                    )
                        .unwrap();

                    tracing::error!(socket_addr = ?socket_addr, already_taken_name = %hello_command.name, "Worker requested to be registered as a new worker, but the requested name is already taken");
                } else {
                    let proto::HelloCommand { name } = hello_command;

                    clients.insert(socket_addr, name.clone());

                    send_response_tx.send(protobuf::master::HelloCommandResponse::ok(name.clone()));

                    tracing::info!(socket_addr = ?socket_addr, requested_name = ?name, "Worker requested to be registered as a new worker, we have accepted him");
                }
            }
            _ => unimplemented!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use proto::{master_client::MasterClient, HelloCommand, WorkerPacket};
    use tokio::{
        sync::{broadcast, mpsc},
        task::JoinHandle,
    };
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_util::sync::CancellationToken;
    use tonic::transport::Channel;
    use proto::worker_packet::Msg as WorkerPacketMsg;

    use crate::{start_grpc_listener, start_respond_on_commands_worker, MasterServer};

    struct MasterTesting {
        cancellation_token: CancellationToken,
        grpc_fut: JoinHandle<Option<Result<(), tonic::transport::Error>>>,
        respond_to_commands_fut: JoinHandle<()>,
        _client: MasterClient<Channel>,
        client_input_tx: mpsc::Sender<WorkerPacket>,
        receive_request_rx: broadcast::Receiver<(core::net::SocketAddr, WorkerPacket)>,
    }

    impl MasterTesting {
        pub async fn new() -> Self {
            let cancellation_token = CancellationToken::new();
            // set up listener
            let (send_response_tx, send_response_rx) = broadcast::channel(128);
            let (receive_request_tx, receive_request_rx) = broadcast::channel(128);

            let (client_input_tx, client_input_rx) = mpsc::channel::<WorkerPacket>(128);

            let server = MasterServer {
                cancellation_token: cancellation_token.clone(),
                send_response_tx: send_response_tx.clone(),
                send_response_rx,
                receive_request_tx: receive_request_tx.clone(),
            };

            let grpc_fut = start_grpc_listener(cancellation_token.clone(), server).await;
            let respond_to_commands_fut = start_respond_on_commands_worker(
                cancellation_token.clone(),
                receive_request_tx.subscribe(),
                send_response_tx,
            );

            // set up stub worker

            let mut client = proto::master_client::MasterClient::connect("http://127.0.0.1:50051")
                .await
                .unwrap();

            let _ = client
                .stream(ReceiverStream::new(client_input_rx))
                .await
                .unwrap();

            Self {
                cancellation_token,
                grpc_fut,
                respond_to_commands_fut,
                _client: client,
                client_input_tx,
                receive_request_rx,
            }
        }

        pub async fn send_packet(self, packet: proto::WorkerPacket) -> Self {
            self.client_input_tx
                .send(packet)
                .await
                .unwrap();

            self
        }

        pub async fn assert_if_identical_packet_has_been_received_on_master(
            mut self,
            expected_packet: proto::WorkerPacket,
        ) -> Self {
            let event = tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    assert!(false, "Time exceeded.");
                    unreachable!();
                }
                event = self.receive_request_rx.recv() => event
            };

            assert!(
                matches!(
                    &event,
                    Ok((_, packet)) if *packet == expected_packet 
                ),
                "Expected to receive identical HelloCommand {expected_packet:?}, received {event:?} instead."
            );

            self
        }

        pub async fn cancel(self) {
            self.cancellation_token.cancel();

            let results = tokio::join!(self.grpc_fut, self.respond_to_commands_fut);

            let _ = results.0.unwrap();
            results.1.unwrap();
        }
    }

    #[tokio::test]
    pub async fn test_receives_hello_command() {
        let packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("helloworld") }))
        };

        MasterTesting::new()
            .await
            .send_packet(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet)
            .await
            .cancel()
            .await;
    }
}
