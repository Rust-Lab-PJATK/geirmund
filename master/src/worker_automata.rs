use std::net::SocketAddr;

use tokio_util::sync::CancellationToken;
use tracing::Level;

use crate::{protobuf, Channel, State};

pub struct WorkerAutomata {
    external_cancellation_token: CancellationToken,

    socket_addr: SocketAddr,
    master_state: State,
    automata_state: WorkerAutomataState,

    receive_worker_packets_channel: Channel<proto::WorkerPacket>,
    send_master_packets_channel: Channel<proto::MasterPacket>,

    disconnect_from_worker_channel: Channel<()>,

    change_state_channel: Channel<WorkerAutomataState>,
}

#[derive(Debug, Clone, Copy)]
pub enum WorkerAutomataState {
    WaitingForHelloCommand,
    WaitingForLoadCommandResponse,
    WaitingForGenerateCommandResponse,
}

impl WorkerAutomata {
    pub fn new(
        // External cancellation token
        cancellation_token: CancellationToken,

        // Socket address of the connected worker
        socket_addr: SocketAddr,
        // Master global state
        master_state: State,

        // Broadcast what am I doing channel
        change_state_channel: Channel<WorkerAutomataState>,

        // Communication channel with socket
        receive_worker_packets_channel: Channel<proto::WorkerPacket>,
        send_master_packets_channel: Channel<proto::MasterPacket>,

        // Channel that you can send a request to disconnect the worker
        disconnect_from_worker_channel: Channel<()>,
    ) -> Self {
        Self {
            socket_addr,

            master_state,

            automata_state: WorkerAutomataState::WaitingForHelloCommand,

            change_state_channel,
            external_cancellation_token: cancellation_token,
            receive_worker_packets_channel,
            send_master_packets_channel,
            disconnect_from_worker_channel,
        }
    }

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                _ = self.external_cancellation_token.cancelled() => {
                    self.disconnect_from_worker_channel.sender().send(());
                    return;
                },
                maybe_worker_packet = self.receive_worker_packets_channel.receiver().recv() => {
                    match maybe_worker_packet {
                        Ok(worker_packet) => self.react_to_new_worker_packet(worker_packet).await,
                        Err(e) => {
                            tracing::error!("Received recoverable error from receive_worker_packets_channel: {e}");
                        }
                    }
                }
            }
        }
    }

    async fn react_to_new_worker_packet(&mut self, worker_packet: proto::WorkerPacket) {
        tracing::event!(Level::TRACE, ?worker_packet, "New worker packet received");

        match self.automata_state {
            WorkerAutomataState::WaitingForHelloCommand => {
                let hello_command = match worker_packet {
                    proto::WorkerPacket {
                        msg: Some(proto::worker_packet::Msg::HelloCommand(hello_command)),
                    } => hello_command,
                    _ => {
                        tracing::event!(Level::ERROR, ?worker_packet, "Received invalid packet, expected proto::HelloCommand. Sending error message and resetting state to WaitingForHelloCommand.");

                        self.send_error(proto::master_error::Msg::UnexpectedPacketReceived(
                            "Expected HelloCommand, resetting state to WaitingForHelloCommand."
                                .to_string(),
                        ));

                        self.change_state(WorkerAutomataState::WaitingForHelloCommand);

                        return;
                    }
                };

                self.react_to_hello_command(hello_command).await;
            }
            _ => unimplemented!(),
        }
    }

    async fn send_error(&mut self, error: proto::master_error::Msg) {
        self.send_master_packets_channel
            .sender()
            .send(proto::MasterPacket {
                msg: Some(proto::master_packet::Msg::Error(proto::MasterError {
                    msg: Some(error.clone()),
                })),
            })
            .unwrap();

        tracing::event!(Level::ERROR, ?error, "Sending error");
    }

    fn change_state(&mut self, state: WorkerAutomataState) {
        self.automata_state = state;
        self.change_state_channel.sender().send(state).unwrap();

        tracing::event!(Level::DEBUG, ?state, "Change of state");
    }

    // react_to_new_worker_packet extensions
    async fn react_to_hello_command(&mut self, hello_command: proto::HelloCommand) {
        tracing::event!(Level::DEBUG, %hello_command.name, "Received hello command");

        if let Some(current_name) = self
            .master_state
            .get_worker_name_by_socket_addr(&self.socket_addr)
            .await
        {
            self.send_master_packets_channel
                .sender()
                .send(
                    protobuf::master::HelloCommandResponse::you_already_have_a_name_error(
                        current_name.clone(),
                    ),
                )
                .unwrap();

            tracing::event!(Level::ERROR, current_name = %current_name, requested_worker_name = %hello_command.name, "Worker requested to be registered as a new worker, but he is already registered");
        } else if self
            .master_state
            .get_socket_addr_by_worker_name(hello_command.name.clone())
            .await
            .is_some()
        {
            self.send_master_packets_channel
                .sender()
                .send(
                    protobuf::master::HelloCommandResponse::worker_with_given_name_already_exists(
                        hello_command.name.clone(),
                    ),
                )
                .unwrap();

            tracing::event!(Level::ERROR, socket_addr = ?self.socket_addr, already_taken_name = %hello_command.name, "Worker requested to be registered as a new worker, but the requested name is already taken");
        } else {
            let proto::HelloCommand { name } = hello_command;

            self.master_state
                .add_worker_name_to_socket_addr_mapping(self.socket_addr, name.clone());

            self.send_master_packets_channel
                .sender()
                .send(protobuf::master::HelloCommandResponse::ok(name.clone()))
                .unwrap();

            tracing::event!(Level::INFO, requested_name = ?name, "Worker requested to be registered as a new worker, we have accepted him.");
        }
    }
}
