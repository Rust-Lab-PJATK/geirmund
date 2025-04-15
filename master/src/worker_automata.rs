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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerAutomataState {
    WaitingForHelloCommand,
    WaitingToSendLoadCommand(proto::ModelType),
    WaitingForLoadCommandResponse(proto::ModelType),
    WaitingToSendGenerateCommand(String),
    WaitingForGenerateCommandResponse { previous_prompt: String },
    End,
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
                    if matches!(self.automata_state, WorkerAutomataState::End) {
                        self.disconnect_from_worker_channel.sender().send(());
                        return;
                    }

                    match maybe_worker_packet {
                        Ok(worker_packet) => self.react_to_new_worker_packet(worker_packet).await,
                        Err(e) => {
                            tracing::error!("Received recoverable error from receive_worker_packets_channel: {e}");
                        }
                    }
                },
                _ = self.change_state_channel.receiver().recv() => {
                    if matches!(self.automata_state, WorkerAutomataState::End) {
                        self.disconnect_from_worker_channel.sender().send(());
                        return;
                    }

                    self.react_to_change_of_state().await;
                }
            }
        }
    }

    async fn react_to_change_of_state(&mut self) {
        match self.automata_state.clone() {
            WorkerAutomataState::WaitingToSendLoadCommand(model) => {
                tracing::event!(Level::DEBUG, "Sending load command to worker");

                self.send_master_packets_channel
                    .sender()
                    .send(protobuf::master::load_command(model))
                    .unwrap();

                self.change_state(WorkerAutomataState::WaitingForLoadCommandResponse(
                    model.into(),
                ));
            }
            WorkerAutomataState::WaitingToSendGenerateCommand(prompt) => {
                tracing::event!(Level::DEBUG, "Sending generate command to worker.");

                self.send_master_packets_channel
                    .sender()
                    .send(protobuf::master::generate_command(prompt.clone()))
                    .unwrap();

                self.change_state(WorkerAutomataState::WaitingForGenerateCommandResponse {
                    previous_prompt: prompt,
                });
            }
            _ => {}
        }
    }

    async fn react_to_new_worker_packet(&mut self, worker_packet: proto::WorkerPacket) {
        tracing::event!(Level::TRACE, ?worker_packet, "New worker packet received");

        match self.automata_state.clone() {
            WorkerAutomataState::WaitingForHelloCommand => {
                match worker_packet {
                    proto::WorkerPacket {
                        msg: Some(proto::worker_packet::Msg::HelloCommand(hello_command)),
                    } => self.react_to_hello_command(hello_command).await,
                    _ => self.handle_invalid_packet(
                        worker_packet,
                        String::from("Received invalid packet, expected proto::HelloCommand. Sending error message and resetting state to WaitingForHelloCommand."),
                        "Expected HelloCommand, resetting to WaitingForHelloCommand".to_string(),
                        WorkerAutomataState::WaitingForHelloCommand,
                    ).await
                };
            }
            WorkerAutomataState::WaitingForLoadCommandResponse(model_type) => match worker_packet {
                proto::WorkerPacket {
                    msg: Some(proto::worker_packet::Msg::LoadCommandResponse(loaded_model)),
                } => {
                    let model_type_in_int: i32 = model_type.into();

                    if let Some(loaded_model) = loaded_model.try_into().ok() {
                        self.react_to_load_command_response(loaded_model).await
                    } else {
                        self.send_error(proto::master_error::Msg::WrongModelHasBeenLoaded(format!(
                            "Wrong model has been loaded, expected {model_type_in_int} ({model_type:?}), received {loaded_model:?}",
                        )));

                        self.change_state(WorkerAutomataState::WaitingToSendLoadCommand(
                            model_type,
                        ));
                    }
                }
                _ => {
                    self.handle_invalid_packet(
                        worker_packet,
                        String::from("Received invalid packet, expected LoadCommandResponse. Sending error message and resetting state to WaitingToSendLoadCommand."),
                        "Expected LoadCommandResponse, resetting to WaitingToSendLoadCommand".to_string(),
                        WorkerAutomataState::WaitingToSendLoadCommand(model_type),
                    ).await;
                }
            },
            WorkerAutomataState::WaitingForGenerateCommandResponse { previous_prompt } => {
                match worker_packet {
                    proto::WorkerPacket {
                        msg: Some(proto::worker_packet::Msg::GenerateCommandResponse(response)),
                    } => {
                        self.disconnect_from_worker_channel
                            .sender()
                            .send(())
                            .unwrap();

                        tracing::event!(Level::INFO, "Received response from worker: {response}");
                    }
                    _ => {
                        self.handle_invalid_packet(
                        worker_packet,
                        String::from("Received invalid packet, expected GenerateCommandResponse. Sending error message and resetting state to WaitingToSendGenerateCommand."),
                        "Expected GenerateCommandResponse, resetting to WaitingToSendGenerateCommand".to_string(),
                        WorkerAutomataState::WaitingToSendGenerateCommand(previous_prompt),
                    ).await;
                    }
                }
            }
            _ => unimplemented!(),
        }
    }

    async fn handle_invalid_packet(
        &mut self,
        worker_packet: proto::WorkerPacket,
        log_message: String,
        error_packet_message: String,
        change_state_to: WorkerAutomataState,
    ) {
        tracing::event!(Level::ERROR, ?worker_packet, "{}", log_message);

        self.send_error(proto::master_error::Msg::UnexpectedPacketReceived(
            error_packet_message,
        ));

        self.change_state(change_state_to);
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
        self.automata_state = state.clone();
        self.change_state_channel
            .sender()
            .send(state.clone())
            .unwrap();

        tracing::event!(Level::DEBUG, ?state, "Change of state");
    }

    // react_to_new_worker_packet extensions
    async fn react_to_hello_command(&mut self, hello_command: proto::HelloCommand) {
        assert!(
            self.automata_state == WorkerAutomataState::WaitingForHelloCommand,
            "Expected automata state to be {:?}, but it is {:?}",
            WorkerAutomataState::WaitingForHelloCommand,
            self.automata_state
        );

        tracing::event!(Level::DEBUG, %hello_command.name, "Received hello command");

        if let Some(current_name) = self
            .master_state
            .get_worker_name_by_socket_addr(&self.socket_addr)
            .await
        {
            self.send_error(proto::master_error::Msg::YouAlreadyHaveANameError(format!(
                "You already have a name assigned: {current_name}!"
            )));

            self.change_state(WorkerAutomataState::WaitingForHelloCommand);

            tracing::event!(Level::ERROR, current_name = %current_name, requested_worker_name = %hello_command.name, "Worker requested to be registered as a new worker, but he is already registered");
        } else if self
            .master_state
            .get_socket_addr_by_worker_name(hello_command.name.clone())
            .await
            .is_some()
        {
            self.send_error(proto::master_error::Msg::WorkerWithGivenNameAlreadyExists(
                format!(
                    "Worker with name {} already exists! Choose something else.",
                    hello_command.name
                ),
            ));

            self.change_state(WorkerAutomataState::WaitingForHelloCommand);

            tracing::event!(Level::ERROR, socket_addr = ?self.socket_addr, already_taken_name = %hello_command.name, "Worker requested to be registered as a new worker, but the requested name is already taken");
        } else {
            let proto::HelloCommand { name } = hello_command;

            self.master_state
                .add_worker_name_to_socket_addr_mapping(self.socket_addr, name.clone());

            self.send_master_packets_channel
                .sender()
                .send(protobuf::master::hello_command_ok())
                .unwrap();

            self.change_state(WorkerAutomataState::WaitingToSendLoadCommand(
                proto::ModelType::Llama3v21b,
            ));

            tracing::event!(Level::INFO, requested_name = ?name, "Worker requested to be registered as a new worker, we have accepted him.");
        }
    }

    async fn react_to_load_command_response(&mut self, loaded_model: proto::ModelType) {
        assert!(
            self.automata_state == WorkerAutomataState::WaitingForLoadCommandResponse(loaded_model),
            "Expected automata state to be {:?}, but it is {:?}",
            WorkerAutomataState::WaitingForLoadCommandResponse(loaded_model),
            self.automata_state
        );

        tracing::event!(Level::DEBUG, ?loaded_model, "Received LoadCommandResponse");

        let expected_model = if let WorkerAutomataState::WaitingForLoadCommandResponse(model_type) =
            self.automata_state
        {
            model_type
        } else {
            unreachable!()
        };

        if expected_model != loaded_model {
            self.send_error(proto::master_error::Msg::WrongModelHasBeenLoaded(format!(
                "Expected to load {expected_model:?}, received {loaded_model:?}"
            )));

            self.change_state(WorkerAutomataState::WaitingToSendLoadCommand(
                expected_model,
            ));

            return;
        }

        self.send_master_packets_channel
            .sender()
            .send(protobuf::master::load_command_response_ack(expected_model))
            .unwrap();

        self.change_state(WorkerAutomataState::WaitingToSendGenerateCommand(
            "Tell me something about Polish-Japanese Academy of Information Technology."
                .to_string(),
        ));
    }
}
