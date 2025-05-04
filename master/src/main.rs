use futures::{Stream, StreamExt};
use proto::{MasterPacket, WorkerPacket};
use state::State;
use std::{
    fmt::Debug,
    net::{SocketAddr, ToSocketAddrs},
    pin::Pin,
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{transport::Server, Code, Request, Response, Status, Streaming};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use worker_automata::{WorkerAutomata, WorkerAutomataState};

mod protobuf;
mod state;
mod worker_automata;

pub struct Channel<T: Clone> {
    receiver: tokio::sync::broadcast::Receiver<T>,
    sender: tokio::sync::broadcast::Sender<T>,
}

impl<T: Clone> Clone for Channel<T> {
    fn clone(&self) -> Self {
        let sender = self.sender.clone();

        Self {
            receiver: sender.subscribe(),
            sender,
        }
    }
}

impl<T: Clone> Channel<T> {
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::broadcast::channel(128);

        Self {
            receiver: rx,
            sender: tx,
        }
    }

    pub fn receiver(&mut self) -> &mut tokio::sync::broadcast::Receiver<T> {
        &mut self.receiver
    }

    pub fn sender(&self) -> &tokio::sync::broadcast::Sender<T> {
        &self.sender
    }
}

pub struct MasterServer {
    port: u16,
    cancellation_token: CancellationToken,
    state: State,
    send_response_channel: Channel<(SocketAddr, MasterPacket)>,
    receive_request_channel: Channel<(SocketAddr, WorkerPacket)>,
    please_disconnect_channel: Channel<SocketAddr>,
    change_worker_automata_state_channel: Channel<(SocketAddr, WorkerAutomataState)>,
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

        let _span = tracing::span!(Level::TRACE, "grpc connection", ?socket_addr);
        let _ = _span.enter();

        tracing::event!(Level::INFO, "new connection");

        self.add_worker_to_state(socket_addr).await?;

        // When client disconnects we need to send cancellation to the tokio worker
        // that sends responses to the worker.
        let connection_cancellation_token = CancellationToken::new();

        // Receive request from worker
        let in_stream = request.into_inner();

        tokio::spawn(
            Self::cancel_connection_cancellation_token_if_global_is_cancelled(
                self.cancellation_token.clone(),
                connection_cancellation_token.clone(),
            ),
        );

        tokio::spawn(Self::remove_worker_from_state_worker(
            self.state.clone(),
            connection_cancellation_token.clone(),
            socket_addr,
        ));

        tokio::spawn(MasterServer::receive_request_worker(
            connection_cancellation_token.clone(),
            socket_addr.clone(),
            in_stream,
            self.receive_request_channel.clone(),
        ));

        tokio::spawn(MasterServer::listen_for_disconnect_request_worker(
            connection_cancellation_token.clone(),
            self.please_disconnect_channel.clone(),
            socket_addr.clone(),
        ));

        // WORKER AUTOMATA START

        // Conversion channels
        let local_change_worker_automata_state_channel = Channel::new();

        tokio::spawn(Self::start_worker_channel_converter(
            connection_cancellation_token.clone(),
            local_change_worker_automata_state_channel.clone(),
            self.change_worker_automata_state_channel.clone(),
            socket_addr,
            |worker_automata_state, socket_addr| (socket_addr, worker_automata_state),
        ));

        let local_receive_request_channel = Channel::new();

        tokio::spawn(Self::start_worker_channel_converter(
            connection_cancellation_token.clone(),
            local_receive_request_channel.clone(),
            self.receive_request_channel.clone(),
            socket_addr,
            |worker_packet, socket_addr| (socket_addr, worker_packet),
        ));

        let local_send_response_channel: Channel<MasterPacket> = Channel::new();

        tokio::spawn(Self::start_worker_channel_converter(
            connection_cancellation_token.clone(),
            local_send_response_channel.clone(),
            self.send_response_channel.clone(),
            socket_addr,
            |master_packet, socket_addr| (socket_addr, master_packet),
        ));

        let please_disconnect_me_channel: Channel<()> = Channel::new();

        tokio::spawn(Self::start_worker_channel_converter(
            connection_cancellation_token.clone(),
            please_disconnect_me_channel.clone(),
            self.please_disconnect_channel.clone(),
            socket_addr,
            |_, socket_addr| socket_addr,
        ));

        // final initialization of WA
        let mut worker_automata = WorkerAutomata::new(
            connection_cancellation_token.clone(),
            socket_addr.clone(),
            self.state.clone(),
            local_change_worker_automata_state_channel.clone(),
            local_receive_request_channel.clone(),
            local_send_response_channel.clone(),
            please_disconnect_me_channel.clone(),
        );

        tokio::spawn(async move {
            worker_automata.run().await;
        });

        // Send response to worker
        let (out_stream_tx, out_stream_rx) = mpsc::channel::<Result<MasterPacket, Status>>(128);

        tokio::spawn(MasterServer::send_response_worker(
            self.send_response_channel.clone(),
            out_stream_tx.clone(),
            self.cancellation_token.clone(),
            connection_cancellation_token,
            socket_addr.clone(),
        ));

        let out_stream = ReceiverStream::new(out_stream_rx);

        Ok(Response::new(Box::pin(out_stream) as Self::StreamStream))
    }
}

impl MasterServer {
    async fn add_worker_to_state(&self, socket_addr: SocketAddr) -> Result<(), Status> {
        let mut tx = self.state.start_transaction().await.map_err(|err| {
            tracing::event!(Level::ERROR, %err, "failed to start tranasaction in internal db");
            return Status::new(
                Code::Internal,
                "failed to start tranasaction in internal db",
            );
        })?;

        self.state
            .create_worker(&mut tx, socket_addr)
            .await
            .map_err(|err| {
                tracing::event!(Level::ERROR, %err, "failed to add worker to internal database");
                return Status::new(Code::Internal, "failed to add worker to internal database");
            })?;

        tx.commit().await.map_err(|err| {
            tracing::event!(Level::ERROR, %err, "failed to commit adding worker to internal database");
            return Status::new(Code::Internal, "failed to commit adding worker to internal database");
        })?;

        Ok(())
    }

    async fn cancel_connection_cancellation_token_if_global_is_cancelled(
        global_cancellation_token: CancellationToken,
        connection_cancellation_token: CancellationToken,
    ) {
        tokio::select! {
            _ = global_cancellation_token.cancelled() => {
                connection_cancellation_token.cancel();
            },
            _ = connection_cancellation_token.cancelled() => {}
        };
    }

    // If connection cancellation token is cancelled, then remove worker from the state
    async fn remove_worker_from_state_worker(
        state: State,
        connection_cancellation_token: CancellationToken,
        socket_addr: SocketAddr,
    ) {
        tokio::select! {
            _ = connection_cancellation_token.cancelled() => {
                let mut tx = match state.start_transaction().await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::event!(Level::ERROR, %e, "error occured while trying to open transaction");
                        return;
                    }
                };

                if let Err(e) = state.delete_worker(&mut tx, socket_addr).await {
                    tracing::event!(Level::ERROR, %e, "error occured while trying to delete worker from internal db");
                    return;
                }

                if let Err(e) = tx.commit().await {
                    tracing::event!(Level::ERROR, %e, "error occured while trying to commit transaction");
                    return;
                }
            }
        };
    }

    async fn start_worker_channel_converter<
        T: Clone,
        K: Clone + Debug,
        L: Fn(T, SocketAddr) -> K,
    >(
        cancellation_token: CancellationToken,
        mut source_channel: Channel<T>,
        destination_channel: Channel<K>,
        socket_addr: SocketAddr,
        conversion_lambda: L,
    ) {
        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => {
                    break;
                }
                maybe_event = source_channel.receiver().recv() => match maybe_event {
                    Ok(data) => {
                        destination_channel.sender().send(conversion_lambda(data, socket_addr)).unwrap();
                    }
                    Err(error) => {
                        tracing::event!(Level::ERROR, ?error, "Received an error while trying to convert message from source channel to destination channel");
                    }
                }
            }
        }
    }

    pub async fn listen_for_disconnect_request_worker(
        connected_cancellation_token: CancellationToken,
        mut please_disconnect_channel: Channel<SocketAddr>,
        socket_addr: SocketAddr,
    ) {
        loop {
            tokio::select! {
                _ = connected_cancellation_token.cancelled() => break,
                Ok(requested_socket_addr) = please_disconnect_channel.receiver().recv() => if socket_addr == requested_socket_addr {
                    tracing::event!(Level::TRACE, "received please disconnect for our socket");
                    connected_cancellation_token.cancel();
                }
            }
        }
    }

    async fn receive_request_worker(
        connection_cancellation_token: CancellationToken,
        socket_addr: SocketAddr,
        mut in_stream: Streaming<WorkerPacket>,
        receive_request_channel: Channel<(SocketAddr, WorkerPacket)>,
    ) {
        loop {
            let maybe_event = tokio::select! {
                maybe_event = in_stream.next() => maybe_event,
                _ = connection_cancellation_token.cancelled() => {
                    break
                },
            };

            let event = match maybe_event {
                Some(Ok(event)) => event,
                Some(Err(e)) => {
                    // TODO: No coś z tym trzeba zrobić w końcu xD
                    tracing::event!(
                        Level::ERROR,
                        error = ?e,
                        "GRPC Error received from worker, ignoring."
                    );
                    continue;
                }
                None => {
                    connection_cancellation_token.cancel();
                    break;
                }
            };

            tracing::debug!(event = tracing::field::debug(&event), "Request received");

            receive_request_channel
                .sender()
                .send((socket_addr, event))
                .unwrap();
        }
    }

    async fn send_response_worker(
        mut send_response_channel: Channel<(SocketAddr, MasterPacket)>,
        out_stream_tx: mpsc::Sender<Result<MasterPacket, Status>>,
        cancellation_token: CancellationToken,
        connected_cancellation_token: CancellationToken,
        socket_addr: SocketAddr,
    ) {
        loop {
            let (event_socket_addr, event_data) = tokio::select! {
                received_event = send_response_channel.receiver().recv() => match received_event {
                    Ok(event) => event,
                    Err(e) => {
                        tracing::error!("recoverable error received on local_send_request_receiver on worker address {socket_addr:?}: {e}");
                        continue;
                    }
                },
                _ = cancellation_token.cancelled() => break,
                _ = connected_cancellation_token.cancelled() => break,
            };

            if socket_addr != event_socket_addr {
                continue;
            }

            if let Err(e) = out_stream_tx.send(Ok(event_data)).await {
                tracing::error!("recoverable error received when trying to pass MasterPacket from local_send_request_receiver to tx: {e}");
            }
        }
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

    let state = match State::new().await {
        Ok(state) => state,
        Err(error) => {
            tracing::error!(?error, "error occured while trying to create state");
            std::process::exit(1);
        }
    };

    let send_response_channel = Channel::new();
    let receive_request_channel = Channel::new();
    let please_disconnect_channel = Channel::new();
    let change_worker_automata_state_channel = Channel::new();

    let server = MasterServer {
        port: 50010,
        state: state.clone(),
        cancellation_token: cancellation_token.clone(),
        please_disconnect_channel: please_disconnect_channel.clone(),
        send_response_channel,
        receive_request_channel,
        change_worker_automata_state_channel,
    };

    tracing::info!("Starting GRPC tcp listener... (there will be no confirmation log)");

    let grpc_server_fut = start_grpc_listener(cancellation_token.clone(), server).await;

    let (grpc_server_result,) = tokio::join!(grpc_server_fut,);

    grpc_server_result.unwrap().unwrap().unwrap();
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

    let server_port = server.port;

    let fut = tokio::spawn(async move {
        cancellation_token
            .run_until_cancelled(async {
                ready_tx.send(()).unwrap();

                Server::builder()
                    .add_service(reflection_service)
                    .add_service(proto::master_server::MasterServer::new(server))
                    .serve(
                        format!("0.0.0.0:{}", server_port)
                            .to_socket_addrs()
                            .unwrap()
                            .next()
                            .unwrap(),
                    )
                    .await
            })
            .await
    });

    ready_rx.await.unwrap();

    fut
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use futures::StreamExt;
    use proto::worker_packet::Msg as WorkerPacketMsg;
    use proto::{master_client::MasterClient, HelloCommand, MasterPacket, WorkerPacket};
    use tokio::{sync::mpsc, task::JoinHandle};
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_util::sync::CancellationToken;

    use crate::worker_automata::WorkerAutomataState;
    use crate::{protobuf, start_grpc_listener, Channel, MasterServer, State};

    struct MasterTesting {
        server_port: u16,
        cancellation_token: CancellationToken,
        grpc_fut: JoinHandle<Option<Result<(), tonic::transport::Error>>>,
        change_worker_automata_state_channel: Channel<(SocketAddr, WorkerAutomataState)>,
        send_response_channel: Channel<(SocketAddr, MasterPacket)>,
        please_disconnect_channel: Channel<SocketAddr>,
        receive_request_channel: Channel<(SocketAddr, WorkerPacket)>,
    }

    impl MasterTesting {
        pub async fn new() -> Self {
            let cancellation_token = CancellationToken::new();

            let server_port = 10000 + rand::random::<u16>() % 50000;

            let change_worker_automata_state_channel = Channel::new();
            let send_response_channel = Channel::new();
            let please_disconnect_channel = Channel::new();
            let receive_request_channel = Channel::new();

            let server = MasterServer {
                port: server_port,
                cancellation_token: cancellation_token.clone(),
                state: State::new().await.unwrap(),
                change_worker_automata_state_channel: change_worker_automata_state_channel.clone(),
                send_response_channel: send_response_channel.clone(),
                please_disconnect_channel: please_disconnect_channel.clone(),
                receive_request_channel: receive_request_channel.clone(),
            };

            let grpc_fut = start_grpc_listener(cancellation_token.clone(), server).await;

            Self {
                server_port,
                cancellation_token,
                grpc_fut,
                change_worker_automata_state_channel,
                send_response_channel,
                please_disconnect_channel,
                receive_request_channel,
            }
        }

        pub async fn connect_new_client(mut self, id: i32) -> Self {
            todo!()
        }

        pub async fn send_packet_to_master(
            mut self,
            client_id: i32,
            packet: proto::WorkerPacket,
        ) -> Self {
            todo!()
        }

        pub async fn assert_if_identical_packet_has_been_received_on_master(
            mut self,
            expected_packet: proto::WorkerPacket,
        ) -> Self {
            todo!()
        }

        pub async fn cancel(self) {
            todo!()
        }

        pub async fn assert_if_identical_packet_has_been_received_on_worker(
            mut self,
            client_id: i32,
            packet: MasterPacket,
        ) -> Self {
            todo!()
        }

        pub async fn assert_no_more_packets_received_on_worker(mut self, client_id: i32) -> Self {
            todo!()
        }

        pub async fn connect_worker_and_register_it(
            mut self,
            client_id: i32,
            name: impl Into<String>,
        ) -> Self {
            todo!()
        }
    }

    struct MasterTestingClient {
        client: MasterClient<Channel>,
        send_request_tx: mpsc::Sender<WorkerPacket>,
        receive_response_rx: mpsc::Receiver<MasterPacket>,
    }

    impl MasterTestingClient {
        pub async fn new(cancellation_token: CancellationToken, port: u16) -> Self {
            let (send_request_tx, send_request_rx) = mpsc::channel(128);
            let (receive_response_tx, receive_response_rx) = mpsc::channel(128);

            // set up stub worker

            let mut client =
                proto::master_client::MasterClient::connect(format!("http://127.0.0.1:{}", port))
                    .await
                    .unwrap();

            let response = client
                .stream(ReceiverStream::new(send_request_rx))
                .await
                .unwrap();

            let mut response_stream = response.into_inner();

            let response_cancellation_token = cancellation_token.clone();
            tokio::spawn(async move {
                loop {
                    let message = tokio::select! {
                        _ = response_cancellation_token.cancelled() => break,
                        message = response_stream.next() => message,
                    };

                    let message = match message {
                        Some(Ok(message)) => message,
                        Some(Err(e)) => {
                            // We do a paic here, because it's testing and the errors for our app
                            // should be self-contained in protobuf types
                            panic!("Received GRPC error from master, error: {e:?}");
                        }
                        // connection between the worker and master has probably ended
                        None => break,
                    };

                    receive_response_tx.send(message).await.unwrap();
                }
            });

            Self {
                client,
                send_request_tx,
                receive_response_rx,
            }
        }

        pub async fn send(self, packet: WorkerPacket) -> Self {
            self.send_request_tx.send(packet).await.unwrap();

            self
        }

        pub async fn assert_packet_received(mut self, packet: MasterPacket) -> Self {
            let event = tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    assert!(false, "Time exceeded.");
                    unreachable!();
                }
                event = self.receive_response_rx.recv() => event
            };

            assert!(
                matches!(&event, Some(received_packet) if *received_packet == packet),
                "Expected to receive identical HelloCommand {packet:?}, received {event:?} instead."
            );

            self
        }

        pub async fn assert_no_packets_received(mut self) -> Self {
            match self.receive_response_rx.try_recv() {
                Ok(packet) => {
                    panic!("Expected no packets to be received, but received a {packet:?} instead.")
                }
                Err(mpsc::error::TryRecvError::Empty) => self,
                Err(mpsc::error::TryRecvError::Disconnected) => self,
            }
        }
    }

    #[tokio::test]
    pub async fn test_receives_hello_command() {
        let packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand {
                name: String::from("helloworld"),
            })),
        };

        MasterTesting::new()
            .await
            .connect_new_client(1)
            .await
            .send_packet_to_master(1, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet)
            .await
            .cancel()
            .await;
    }

    #[tokio::test]
    pub async fn test_if_you_already_have_a_name_error_handler_works_on_same_name() {
        let packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand {
                name: String::from("helloworld"),
            })),
        };

        MasterTesting::new()
            .await
            .connect_new_client(1)
            .await
            .send_packet_to_master(1, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                1,
                protobuf::master::HelloCommandResponse::ok("helloworld".to_string()),
            )
            .await
            .send_packet_to_master(1, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                1,
                protobuf::master::HelloCommandResponse::you_already_have_a_name_error(
                    "helloworld".to_string(),
                ),
            )
            .await
            .cancel()
            .await;
    }

    #[tokio::test]
    pub async fn test_if_you_already_have_a_name_error_handler_works_on_different_name() {
        let first_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand {
                name: String::from("first_worker"),
            })),
        };

        let second_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand {
                name: String::from("second_worker"),
            })),
        };

        MasterTesting::new()
            .await
            .connect_new_client(1)
            .await
            .send_packet_to_master(1, first_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(first_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                1,
                protobuf::master::HelloCommandResponse::ok("first_worker".to_string()),
            )
            .await
            .send_packet_to_master(1, second_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(second_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                1,
                protobuf::master::HelloCommandResponse::you_already_have_a_name_error(
                    "first_worker".to_string(),
                ),
            )
            .await
            .cancel()
            .await;
    }

    #[tokio::test]
    pub async fn test_if_worker_with_given_name_already_exists_handler_works() {
        let packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand {
                name: String::from("first_worker"),
            })),
        };

        MasterTesting::new()
            .await
            .connect_new_client(1)
            .await
            .connect_new_client(2)
            .await
            .send_packet_to_master(1, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                1,
                protobuf::master::HelloCommandResponse::ok("first_worker".to_string()),
            )
            .await
            .send_packet_to_master(2, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                2,
                protobuf::master::HelloCommandResponse::worker_with_given_name_already_exists(
                    "first_worker".to_string(),
                ),
            )
            .await
            .cancel()
            .await;
    }

    #[tokio::test]
    pub async fn test_if_master_server_can_register_two_workers() {
        let first_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand {
                name: String::from("first_worker"),
            })),
        };

        let second_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand {
                name: String::from("second_worker"),
            })),
        };

        MasterTesting::new()
            .await
            .connect_new_client(1)
            .await
            .connect_new_client(2)
            .await
            .send_packet_to_master(1, first_packet.clone())
            .await
            .send_packet_to_master(2, second_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(first_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                1,
                protobuf::master::HelloCommandResponse::ok("first_worker".to_string()),
            )
            .await
            .assert_if_identical_packet_has_been_received_on_master(second_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(
                2,
                protobuf::master::HelloCommandResponse::ok("second_worker".to_string()),
            )
            .await
            .cancel()
            .await;
    }
}
