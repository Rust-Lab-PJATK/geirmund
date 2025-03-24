use futures::{Stream, StreamExt};
use proto::{MasterPacket, WorkerPacket};
use std::{collections::HashMap, net::{SocketAddr, ToSocketAddrs}, pin::Pin, sync::Arc};
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
    port: u16,
    cancellation_token: CancellationToken,
    send_response_tx: broadcast::Sender<(SocketAddr, MasterPacket)>,
    send_response_rx: broadcast::Receiver<(SocketAddr, MasterPacket)>,
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
        let (out_stream_tx, out_stream_rx) = mpsc::channel::<Result<MasterPacket, Status>>(128);

        tokio::spawn(MasterServer::send_response_worker(self.send_response_tx.subscribe(), out_stream_tx.clone(), self.cancellation_token.clone(), socket_addr.clone()));

        let out_stream = ReceiverStream::new(out_stream_rx);

        Ok(Response::new(Box::pin(out_stream) as Self::StreamStream))
    }
}

impl MasterServer {
    async fn send_response_worker(mut send_response_rx: broadcast::Receiver<(SocketAddr, MasterPacket)>, out_stream_tx: mpsc::Sender<Result<MasterPacket, Status>>, cancellation_token: CancellationToken, socket_addr: SocketAddr) {
        loop {
            let (event_socket_addr, event_data) = tokio::select! {
                received_event = send_response_rx.recv() => match received_event {
                    Ok(event) => event,
                    Err(e) => {
                        tracing::error!("recoverable error received on local_send_request_receiver on worker address {socket_addr:?}: {e}");
                        continue;
                    }
                },
                _ = cancellation_token.cancelled() => break,
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

    let (send_response_tx, send_response_rx) = broadcast::channel(128);
    let (receive_request_tx, receive_request_rx) = broadcast::channel(128);

    let server = MasterServer {
        port: 50010,
        cancellation_token: cancellation_token.clone(),
        send_response_tx: send_response_tx.clone(),
        send_response_rx,
        receive_request_tx,
    };

    tracing::info!("Starting GRPC tcp listener... (there will be no confirmation log)");

    let grpc_server_fut = start_grpc_listener(cancellation_token.clone(), server).await;

    let state = State::new();

    let respond_on_commands_cancellation_token = cancellation_token.clone();
    let respond_on_commands_fut = start_respond_on_commands_worker(
        state.clone(),
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

    let server_port = server.port;

    let fut = tokio::spawn(async move {
        cancellation_token
            .run_until_cancelled(async {
                ready_tx.send(()).unwrap();

                Server::builder()
                    .add_service(reflection_service)
                    .add_service(proto::master_server::MasterServer::new(server))
                    .serve(format!("0.0.0.0:{}", server_port).to_socket_addrs().unwrap().next().unwrap())
                    .await
            })
            .await
    });

    ready_rx.await.unwrap();

    fut
}

fn start_respond_on_commands_worker(
    state: State,
    cancellation_token: CancellationToken,
    receive_request_rx: broadcast::Receiver<(core::net::SocketAddr, WorkerPacket)>,
    send_response_tx: broadcast::Sender<(SocketAddr, MasterPacket)>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let error_cancellation_token = cancellation_token.clone();

        if let Err(error) = respond_on_commands(state, cancellation_token, receive_request_rx, send_response_tx).await {
            tracing::error!(error = ?error, "Failed to send request to socket handler to send a request with MasterPacket");
            error_cancellation_token.cancel();
        }
    })
}

#[derive(Clone)]
struct WorkerState {
    address: SocketAddr,
    name: Option<String>,
    status: WorkerStatus,
}

#[derive(Default, Clone)]
enum WorkerStatus {
    #[default]
    NotInitialized,
    Registered { name: String },
    WaitingForWorkerToLoadModel { model: String },
    WaitingForMasterToSendInput { loaded_model: String },
    GeneratingResponse { input: String },
}

struct StateShape {
    workers: Vec<WorkerState>,
    change_signal_tx: broadcast::Sender<()>,
}

#[derive(thiserror::Error, Debug)]
#[error("worker with given socket address {socket_addr}, does not exist")]
struct WorkerWithGivenSocketAddressDoesNotExist { socket_addr: SocketAddr }

impl StateShape {
    pub fn new(change_signal_tx: broadcast::Sender<()>) -> Self {
        Self {
            workers: Vec::new(),
            change_signal_tx,
        }
    }

    pub fn add_worker(&mut self, socket_addr: SocketAddr) {
        self.workers.push(WorkerState {
            address: socket_addr,
            name: None,
            status: WorkerStatus::default(),
        });

        self.change_signal_tx.send(());
    }

    pub fn assign_worker_a_name(&mut self, socket_addr: SocketAddr, worker_name: impl Into<String>) -> Result<(), WorkerWithGivenSocketAddressDoesNotExist> {
        let worker_name = worker_name.into();

        let mut position = self.workers.iter().position(|worker| worker.address == socket_addr)
            .ok_or(WorkerWithGivenSocketAddressDoesNotExist { socket_addr })?
            .clone();

        let mut worker = self.workers.remove(position);

        worker.name = Some(worker_name);

        self.workers.push(worker);

        self.change_signal_tx.send(());

        Ok(())
    }

    pub fn get_worker_name_by_socket_addr(&self, socket_addr: &SocketAddr) -> &Option<String> {
        match self.workers.iter().find(|value| value.address == *socket_addr).map(|value| &value.name) {
            Some(worker_state) => worker_state,
            None => &None,
        }
    }

    pub fn get_socket_addr_by_worker_name(&self, worker_name: String) -> Option<&SocketAddr> {
        let position = self.workers.iter().position(|worker| worker.name.as_ref() == Some(&worker_name));

        match position {
            Some(nth) => Some(&self.workers[nth].address),
            None => None,
        }
    }
}

struct State {
    shape: std::sync::Arc<tokio::sync::Mutex<StateShape>>,
    change_signal_tx: broadcast::Sender<()>,
    change_signal_rx: broadcast::Receiver<()>,
}

impl Clone for State {
    fn clone(&self) -> Self {
        Self {
            shape: self.shape.clone(),
            change_signal_tx: self.change_signal_tx.clone(),
            change_signal_rx: self.change_signal_tx.subscribe(),
        }
    }
}

impl State {
    pub fn new() -> Self {
        let (change_signal_tx, change_signal_rx) = broadcast::channel(128);

        Self {
            shape: Arc::new(tokio::sync::Mutex::new(StateShape::new(change_signal_tx.clone()))),
            change_signal_rx,
            change_signal_tx
        }
    }

    pub async fn add_worker_name_to_socket_addr_mapping(&self, socket_addr: SocketAddr, worker_name: impl Into<String>) -> Result<(), WorkerWithGivenSocketAddressDoesNotExist> {
        let shape = self.shape.clone();
        let shape = &mut *shape.lock().await;

        shape.assign_worker_a_name(socket_addr, worker_name)
    }

    pub async fn get_worker_name_by_socket_addr(&self, socket_addr: &SocketAddr) -> Option<String> {
        let shape = self.shape.clone();
        let shape = &mut *shape.lock().await;

        shape.get_worker_name_by_socket_addr(socket_addr).clone()
    }

    pub async fn get_socket_addr_by_worker_name(&self, worker_name: String) -> Option<SocketAddr> {
        let shape = self.shape.clone();
        let shape = &mut *shape.lock().await;

        shape.get_socket_addr_by_worker_name(worker_name).cloned()
    }

    pub async fn workers(&self) -> Vec<WorkerState> {
        let shape = self.shape.clone();
        let shape = &mut *shape.lock().await;

        shape.workers.clone()
    }

    pub async fn changed(&mut self) {
        loop {
            if let Err(e) = self.change_signal_rx.recv().await {
                tracing::debug!(error = ?e, "Recoverable (and probably not imporatant) error received while waiting on change_signal_rx");
            } else {
                break;
            }
        }
    }
}

async fn respond_on_commands(
    state: State,
    cancellation_token: CancellationToken,
    mut receive_request_rx: broadcast::Receiver<(core::net::SocketAddr, WorkerPacket)>,
    send_response_tx: broadcast::Sender<(SocketAddr, MasterPacket)>,
) -> Result<(), broadcast::error::SendError<(SocketAddr, MasterPacket)>> {
    loop {
        let request = tokio::select! {
            received_req = receive_request_rx.recv() => match received_req {
                Ok(received_req) => received_req,
                Err(e) => {
                    tracing::error!(error = tracing::field::debug(e), "Recoverable error received while listening for new requests");
                    continue;
                },
            },
            _ = cancellation_token.cancelled() => return Ok(()),
        };

        let (socket_addr, _) = request;

        match request.1 {
            WorkerPacket {
                msg: Some(proto::worker_packet::Msg::HelloCommand(hello_command)),
            } => respond_on_hello_command(state.clone(), socket_addr, hello_command, send_response_tx.clone()).await?,
            _ => unimplemented!(),
        }
    }
}

async fn respond_on_hello_command(
    state: State, 
    socket_addr: SocketAddr, 
    hello_command: proto::HelloCommand, 
    send_response_tx: broadcast::Sender<(SocketAddr, MasterPacket)>
) -> Result<(), broadcast::error::SendError<(SocketAddr, MasterPacket)>> {
    tracing::info!(socket_addr = ?socket_addr, requested_worker_name = %hello_command.name, "Received Hello! from worker");

    if let Some(current_name) = state.get_worker_name_by_socket_addr(&socket_addr).await {
        send_response_tx.send((
            socket_addr,
            protobuf::master::HelloCommandResponse::you_already_have_a_name_error(current_name.clone())
        ))?;

        tracing::error!(socket_addr = ?socket_addr, current_name = %current_name, requested_worker_name = %hello_command.name, "Worker requested to be registered as a new worker, but he is already registered");
    } else if state.get_socket_addr_by_worker_name(&hello_command.name).await.is_some() {
        send_response_tx.send((
            socket_addr,
            protobuf::master::HelloCommandResponse::worker_with_given_name_already_exists(hello_command.name.clone())
        ))?;

        tracing::error!(socket_addr = ?socket_addr, already_taken_name = %hello_command.name, "Worker requested to be registered as a new worker, but the requested name is already taken");
    } else {
        let proto::HelloCommand { name } = hello_command;

        state.add_worker_name_to_socket_addr_mapping(socket_addr, name.clone()).await;

        send_response_tx.send((socket_addr, protobuf::master::HelloCommandResponse::ok(name.clone())))?;

        tracing::info!(socket_addr = ?socket_addr, requested_name = ?name, "Worker requested to be registered as a new worker, we have accepted him");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use futures::StreamExt;
    use proto::{master_client::MasterClient, HelloCommand, MasterPacket, WorkerPacket};
    use tokio::{
        sync::{broadcast, mpsc},
        task::JoinHandle,
    };
    use tokio_stream::wrappers::ReceiverStream;
    use tokio_util::sync::CancellationToken;
    use tonic::transport::Channel;
    use proto::worker_packet::Msg as WorkerPacketMsg;

    use crate::{protobuf, start_grpc_listener, start_respond_on_commands_worker, MasterServer, State};

    struct MasterTesting {
        server_port: u16,
        cancellation_token: CancellationToken,
        grpc_fut: JoinHandle<Option<Result<(), tonic::transport::Error>>>,
        respond_to_commands_fut: JoinHandle<()>,
        receive_request_rx: broadcast::Receiver<(core::net::SocketAddr, WorkerPacket)>,
        clients: HashMap<i32, MasterTestingClient>
    }

    impl MasterTesting {
        pub async fn new() -> Self {
            let cancellation_token = CancellationToken::new();
            // set up listener
            let (send_response_tx, send_response_rx) = broadcast::channel(128);
            let (receive_request_tx, receive_request_rx) = broadcast::channel(128);

            let server_port = 10000 + rand::random::<u16>() % 50000;

            let server = MasterServer {
                port: server_port,
                cancellation_token: cancellation_token.clone(),
                send_response_tx: send_response_tx.clone(),
                send_response_rx,
                receive_request_tx: receive_request_tx.clone(),
            };

            let grpc_fut = start_grpc_listener(cancellation_token.clone(), server).await;
            let state = State::new();
            let respond_to_commands_fut = start_respond_on_commands_worker(
                state,
                cancellation_token.clone(),
                receive_request_tx.subscribe(),
                send_response_tx,
            );

            Self {
                server_port,
                cancellation_token,
                grpc_fut,
                respond_to_commands_fut,
                receive_request_rx,
                clients: HashMap::new(),
            }
        }

        pub async fn connect_new_client(mut self, id: i32) -> Self{
            if self.clients.contains_key(&id) {
                panic!("Client with id {id} is already connected.");
            }

            self.clients.insert(id, MasterTestingClient::new(self.cancellation_token.clone(), self.server_port).await);

            self
        }

        pub async fn send_packet_to_master(mut self, client_id: i32, packet: proto::WorkerPacket) -> Self {
            let client = self.clients.remove(&client_id)
                .unwrap()
                .send(packet)
                .await;

            self.clients.insert(client_id, client);

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

        pub async fn assert_if_identical_packet_has_been_received_on_worker(mut self, client_id: i32, packet: MasterPacket) -> Self {
            let client = self.clients.remove(&client_id).unwrap().assert_packet_received(packet).await;
            self.clients.insert(client_id, client);

            self
        }


        pub async fn assert_no_more_packets_received_on_worker(mut self, client_id: i32) -> Self {
            let client = self.clients.remove(&client_id).unwrap().assert_no_packets_received().await;
            self.clients.insert(client_id, client);

            self
        }

        pub async fn connect_worker_and_register_it(mut self, client_id: i32, name: impl Into<String>) -> Self {
            let name = name.into();

            let request_packet = protobuf::worker::HelloCommand::new(name.clone());
            let response_packet = protobuf::master::HelloCommandResponse::ok(name);

            self.connect_new_client(client_id).await
                .send_packet_to_master(client_id, request_packet.clone()).await
                .assert_if_identical_packet_has_been_received_on_master(request_packet).await
                .assert_if_identical_packet_has_been_received_on_worker(client_id, response_packet).await
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

            let mut client = proto::master_client::MasterClient::connect(format!("http://127.0.0.1:{}", port))
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
                        },
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

        pub async fn assert_packet_received(mut self, packet: MasterPacket) -> Self{
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

        pub async fn assert_no_packets_received(mut self) -> Self{
            match self.receive_response_rx.try_recv() {
                Ok(packet) => panic!("Expected no packets to be received, but received a {packet:?} instead."),
                Err(mpsc::error::TryRecvError::Empty) => self,
                Err(mpsc::error::TryRecvError::Disconnected) => self,
            }
        }
    }

    #[tokio::test]
    pub async fn test_receives_hello_command() {
        let packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("helloworld") }))
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
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("helloworld") }))
        };

        MasterTesting::new()
            .await
            .connect_new_client(1)
            .await
            .send_packet_to_master(1, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(1, protobuf::master::HelloCommandResponse::ok("helloworld".to_string()))
            .await
            .send_packet_to_master(1, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(1, protobuf::master::HelloCommandResponse::you_already_have_a_name_error("helloworld".to_string()))
            .await
            .cancel()
        .await;
    }

    #[tokio::test]
    pub async fn test_if_you_already_have_a_name_error_handler_works_on_different_name() {
        let first_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("first_worker") }))
        };

        let second_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("second_worker") }))
        };

        MasterTesting::new()
            .await
            .connect_new_client(1)
            .await
            .send_packet_to_master(1, first_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(first_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(1, protobuf::master::HelloCommandResponse::ok("first_worker".to_string()))
            .await
            .send_packet_to_master(1, second_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(second_packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(1, protobuf::master::HelloCommandResponse::you_already_have_a_name_error("first_worker".to_string()))
            .await
            .cancel()
        .await;
    }

    #[tokio::test]
    pub async fn test_if_worker_with_given_name_already_exists_handler_works() {
        let packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("first_worker") }))
        };

        MasterTesting::new()
            .await
            .connect_new_client(1).await
            .connect_new_client(2).await
            .send_packet_to_master(1, packet.clone()).await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone()).await
            .assert_if_identical_packet_has_been_received_on_worker(1, protobuf::master::HelloCommandResponse::ok("first_worker".to_string()))
            .await
            .send_packet_to_master(2, packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_master(packet.clone())
            .await
            .assert_if_identical_packet_has_been_received_on_worker(2, protobuf::master::HelloCommandResponse::worker_with_given_name_already_exists("first_worker".to_string()))
            .await
            .cancel()
            .await;
    }

    #[tokio::test]
    pub async fn test_if_master_server_can_register_two_workers() {
        let first_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("first_worker") }))
        };

        let second_packet = WorkerPacket {
            msg: Some(WorkerPacketMsg::HelloCommand(HelloCommand { name: String::from("second_worker") }))
        };

        MasterTesting::new()
            .await
            .connect_new_client(1).await
            .connect_new_client(2).await
            .send_packet_to_master(1, first_packet.clone()).await
            .send_packet_to_master(2, second_packet.clone()).await
            .assert_if_identical_packet_has_been_received_on_master(first_packet.clone()).await
            .assert_if_identical_packet_has_been_received_on_worker(1, protobuf::master::HelloCommandResponse::ok("first_worker".to_string())).await
            .assert_if_identical_packet_has_been_received_on_master(second_packet.clone()).await
            .assert_if_identical_packet_has_been_received_on_worker(2, protobuf::master::HelloCommandResponse::ok("second_worker".to_string()))
            .await
            .cancel()
            .await;
    }
}
