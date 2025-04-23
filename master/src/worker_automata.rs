use std::net::SocketAddr;

use error::{ReactToHelloCommandError, ReactToNewWorkerPacketError, RollbackStateError};
use proto::ModelType;
use tokio_util::sync::CancellationToken;
use tracing::Level;

use crate::state::{State, StateTransaction};
use crate::{protobuf, Channel};

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
    WaitingForLoadCommandResponse,
    WaitingToSendGenerateCommand,
    WaitingForGenerateCommandResponse { generation_query_id: i64 },
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
                        self.external_cancellation_token.cancel();
                        return;
                    }

                    match maybe_worker_packet {
                        Ok(worker_packet) => if let Err(e) = self.react_to_new_worker_packet(worker_packet).await {
                            tracing::event!(Level::ERROR, error = %e, "unrecoverable error received from react_to_new_worker_packet");

                            self.disconnect_from_worker_channel.sender().send(());
                            self.external_cancellation_token.cancel();
                        },
                        Err(e) => {
                            tracing::error!("Received recoverable error from receive_worker_packets_channel: {e}");
                        }
                    }
                },
                _ = self.change_state_channel.receiver().recv() => {
                    if matches!(self.automata_state, WorkerAutomataState::End) {
                        self.disconnect_from_worker_channel.sender().send(());
                        self.external_cancellation_token.cancel();
                        return;
                    }

                    if let Err(e) = self.react_to_change_of_state().await {
                        tracing::event!(Level::ERROR, error = %e, "unrecoverable error received from react_to_change_of_state");
                        self.disconnect_from_worker_channel.sender().send(());
                        self.external_cancellation_token.cancel();
                    }
                }
            }
        }
    }

    async fn react_to_change_of_state(&mut self) -> Result<(), error::ReactToChangeOfStateError> {
        match self.automata_state.clone() {
            WorkerAutomataState::WaitingToSendLoadCommand(model) => {
                tracing::event!(Level::DEBUG, "Sending load command to worker");

                let mut tx = self.master_state.start_transaction().await?;

                self.master_state
                    .set_model(&mut tx, Some(model), self.socket_addr)
                    .await?;

                self.master_state
                    .set_is_loading_model_status(&mut tx, true, self.socket_addr)
                    .await?;

                tx.commit().await?;

                self.send_master_packets_channel
                    .sender()
                    .send(protobuf::master::load_command(model))
                    .unwrap();

                self.change_state(WorkerAutomataState::WaitingForLoadCommandResponse);
            }
            WorkerAutomataState::WaitingToSendGenerateCommand => {
                tracing::event!(Level::DEBUG, "Sending generate command to worker.");

                let prompt =
                    "Tell me something about Polish-Japanese Academy of Information Technology."
                        .to_string();

                let mut tx = self.master_state.start_transaction().await?;

                let query_id = self
                    .master_state
                    .create_generation_query(&mut tx, self.socket_addr, prompt.clone())
                    .await?;

                tx.commit().await?;

                self.send_master_packets_channel
                    .sender()
                    .send(protobuf::master::generate_command(prompt))
                    .unwrap();

                self.change_state(WorkerAutomataState::WaitingForGenerateCommandResponse {
                    generation_query_id: query_id,
                });
            }
            _ => {}
        }

        Ok(())
    }

    async fn react_to_new_worker_packet(
        &mut self,
        worker_packet: proto::WorkerPacket,
    ) -> Result<(), error::ReactToNewWorkerPacketError> {
        tracing::event!(Level::TRACE, ?worker_packet, "New worker packet received");

        match self.automata_state.clone() {
            WorkerAutomataState::WaitingForHelloCommand => {
                match worker_packet {
                    proto::WorkerPacket {
                        msg: Some(proto::worker_packet::Msg::HelloCommand(hello_command)),
                    } => self.react_to_hello_command(hello_command).await?,
                    _ => self.handle_invalid_packet(
                        worker_packet,
                        String::from("Received invalid packet, expected proto::HelloCommand. Sending error message and resetting state to WaitingForHelloCommand."),
                        "Expected HelloCommand, resetting to WaitingForHelloCommand".to_string(),
                        WorkerAutomataState::WaitingForHelloCommand,
                    ).await?,
                };
            }
            WorkerAutomataState::WaitingForLoadCommandResponse => match worker_packet {
                proto::WorkerPacket {
                    msg: Some(proto::worker_packet::Msg::LoadCommandResponse(loaded_model)),
                } => {
                    if let Some(loaded_model) = loaded_model.try_into().ok() {
                        self.react_to_load_command_response(loaded_model).await?;
                    } else {
                        let mut tx = self.master_state.start_transaction().await?;

                        let worker = self
                            .master_state
                            .get_worker_by_socket_addr(&mut tx, self.socket_addr)
                            .await?
                            .ok_or(ReactToNewWorkerPacketError::WorkerDoesNotExistInDatabase(
                                self.socket_addr,
                            ))?;

                        self.rollback_state(
                            &mut tx,
                            WorkerAutomataState::WaitingToSendLoadCommand(worker.model().unwrap()),
                        )
                        .await?;

                        tx.commit().await?;

                        self.send_error(proto::master_error::Msg::WrongModelHasBeenLoaded(
                            format!(
                            "Wrong model has been loaded, expected {:?}, received {loaded_model:?}",
                            worker.model().unwrap()
                        ),
                        ));
                    }
                }
                _ => {
                    let mut tx = self.master_state.start_transaction().await?;

                    let model = self
                        .master_state
                        .get_worker_by_socket_addr(&mut tx, self.socket_addr)
                        .await?
                        .map(|worker| worker.model().unwrap_or(ModelType::Llama3v21b))
                        .unwrap_or(ModelType::Llama3v21b);

                    tx.rollback();

                    self.handle_invalid_packet(
                        worker_packet,
                        String::from("Received invalid packet, expected LoadCommandResponse. Sending error message and resetting state to WaitingToSendLoadCommand."),
                        "Expected LoadCommandResponse, resetting to WaitingToSendLoadCommand".to_string(),
                        WorkerAutomataState::WaitingToSendLoadCommand(model),
                    ).await;
                }
            },
            WorkerAutomataState::WaitingForGenerateCommandResponse {
                generation_query_id: _,
            } => match worker_packet {
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
                        WorkerAutomataState::WaitingToSendGenerateCommand,
                    ).await;
                }
            },
            _ => {
                self.handle_invalid_packet(
                    worker_packet,
                    String::from(
                        "Received unwanted packet. Sending error message and resetting state to .",
                    ),
                    "Expected GenerateCommandResponse, resetting to WaitingToSendGenerateCommand"
                        .to_string(),
                    WorkerAutomataState::WaitingForHelloCommand,
                )
                .await;
            }
        };

        Ok(())
    }

    async fn handle_invalid_packet(
        &mut self,
        worker_packet: proto::WorkerPacket,
        log_message: String,
        error_packet_message: String,
        change_state_to: WorkerAutomataState,
    ) -> Result<(), RollbackStateError> {
        tracing::event!(Level::ERROR, ?worker_packet, "{}", log_message);

        self.send_error(proto::master_error::Msg::UnexpectedPacketReceived(
            error_packet_message,
        ));

        let mut tx = self.master_state.start_transaction().await?;

        self.rollback_state(&mut tx, change_state_to).await?;

        tx.commit().await?;

        Ok(())
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

    async fn rollback_state<'a>(
        &mut self,
        mut tx: &mut StateTransaction<'a>,
        state: WorkerAutomataState,
    ) -> Result<(), RollbackStateError> {
        match state {
            WorkerAutomataState::WaitingForHelloCommand => {
                self.master_state
                    .set_model(&mut tx, None, self.socket_addr)
                    .await
                    .map_err(|err| RollbackStateError::SetModelError(err))?;

                self.master_state
                    .set_is_loading_model_status(&mut tx, false, self.socket_addr)
                    .await
                    .map_err(|err| RollbackStateError::SetLoadingModelStatusError(err))?;

                self.master_state
                    .assign_name_to_worker(&mut tx, self.socket_addr, None)
                    .await
                    .map_err(|err| RollbackStateError::AssignNameToWorker(err))?;
            }
            WorkerAutomataState::WaitingToSendLoadCommand(_) => {
                self.master_state
                    .set_model(&mut tx, None, self.socket_addr)
                    .await
                    .map_err(|err| RollbackStateError::SetModelError(err))?;

                self.master_state
                    .set_is_loading_model_status(&mut tx, false, self.socket_addr)
                    .await
                    .map_err(|err| RollbackStateError::SetLoadingModelStatusError(err))?;
            }
            WorkerAutomataState::WaitingForLoadCommandResponse => {
                self.master_state
                    .set_is_loading_model_status(&mut tx, true, self.socket_addr)
                    .await
                    .map_err(|err| RollbackStateError::SetLoadingModelStatusError(err))?;
            }
            WorkerAutomataState::WaitingToSendGenerateCommand => {}
            WorkerAutomataState::WaitingForGenerateCommandResponse {
                generation_query_id: _,
            } => {}
            WorkerAutomataState::End => {}
        }

        self.automata_state = state.clone();
        self.change_state_channel
            .sender()
            .send(state.clone())
            .unwrap();

        tracing::event!(Level::DEBUG, ?state, "Change of state");

        Ok(())
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
    async fn react_to_hello_command(
        &mut self,
        hello_command: proto::HelloCommand,
    ) -> Result<(), error::ReactToHelloCommandError> {
        assert!(
            self.automata_state == WorkerAutomataState::WaitingForHelloCommand,
            "Expected automata state to be {:?}, but it is {:?}",
            WorkerAutomataState::WaitingForHelloCommand,
            self.automata_state
        );

        tracing::event!(Level::DEBUG, %hello_command.name, "Received hello command");

        let mut tx = self.master_state.start_transaction().await?;

        let worker_state = self
            .master_state
            .get_worker_by_socket_addr(&mut tx, self.socket_addr)
            .await?
            .ok_or(ReactToHelloCommandError::WorkerDoesNotExistInDatabase(
                self.socket_addr,
            ))?;

        let worker_name = worker_state.name();

        if worker_name.is_some() {
            self.rollback_state(&mut tx, WorkerAutomataState::WaitingForHelloCommand)
                .await?;

            tx.commit().await?;

            self.send_error(proto::master_error::Msg::YouAlreadyHaveANameError(format!(
                "You already have a name assigned: {}!",
                worker_name.clone().unwrap()
            )));

            tracing::event!(Level::ERROR, current_name = %worker_name.unwrap(), requested_worker_name = %hello_command.name, "Worker requested to be registered as a new worker, but he is already registered");
        } else if self
            .master_state
            .get_worker_by_name(&mut tx, &hello_command.name)
            .await?
            .is_some()
        {
            self.rollback_state(&mut tx, WorkerAutomataState::WaitingForHelloCommand)
                .await?;

            tx.commit().await?;

            self.send_error(proto::master_error::Msg::WorkerWithGivenNameAlreadyExists(
                format!(
                    "Worker with name {} already exists! Choose something else.",
                    hello_command.name
                ),
            ));

            tracing::event!(Level::ERROR, socket_addr = ?self.socket_addr, already_taken_name = %hello_command.name, "Worker requested to be registered as a new worker, but the requested name is already taken");
        } else {
            let proto::HelloCommand { name } = hello_command;

            self.master_state
                .assign_name_to_worker(&mut tx, self.socket_addr, Some(name.clone()))
                .await?;

            self.send_master_packets_channel
                .sender()
                .send(protobuf::master::hello_command_ok())
                .unwrap();

            tx.commit().await?;

            self.change_state(WorkerAutomataState::WaitingToSendLoadCommand(
                proto::ModelType::Llama3v21b,
            ));

            tracing::event!(Level::INFO, requested_name = ?name, "Worker requested to be registered as a new worker, we have accepted him.");
        }

        Ok(())
    }

    async fn react_to_load_command_response(
        &mut self,
        loaded_model: proto::ModelType,
    ) -> Result<(), error::ReactToLoadCommandResponseError> {
        assert!(
            self.automata_state == WorkerAutomataState::WaitingForLoadCommandResponse,
            "Expected automata state to be {:?}, but it is {:?}",
            WorkerAutomataState::WaitingForLoadCommandResponse,
            self.automata_state
        );

        tracing::event!(Level::DEBUG, ?loaded_model, "Received LoadCommandResponse");

        let mut tx = self.master_state.start_transaction().await?;

        let worker = self
            .master_state
            .get_worker_by_socket_addr(&mut tx, self.socket_addr)
            .await?
            .ok_or(
                error::ReactToLoadCommandResponseError::WorkerDoesNotExistInDatabase(
                    self.socket_addr,
                ),
            )?;

        let expected_model = worker.model().unwrap();

        if expected_model != loaded_model {
            tx.rollback();

            self.send_error(proto::master_error::Msg::WrongModelHasBeenLoaded(format!(
                "Expected to load {expected_model:?}, received {loaded_model:?}"
            )));

            self.change_state(WorkerAutomataState::WaitingToSendLoadCommand(
                expected_model,
            ));

            return Ok(());
        }

        self.send_master_packets_channel
            .sender()
            .send(protobuf::master::load_command_response_ack(expected_model))
            .unwrap();

        self.change_state(WorkerAutomataState::WaitingToSendGenerateCommand);

        Ok(())
    }
}

pub mod error {
    use crate::state::error::{
        AssignNameToWorkerError, CreateGenerationQueryError, GetError, SetLoadingModelStatusError,
        SetModelError,
    };
    use std::net::SocketAddr;

    #[derive(thiserror::Error, Debug)]
    pub enum ReactToHelloCommandError {
        #[error("database error has occured: {0}")]
        DatabaseError(#[from] sqlx::Error),

        #[error("error occured while trying to get worker data: {0}")]
        GetWorkerError(#[from] crate::state::error::GetError),

        #[error("worker with socket addr {0} does not exist inside internal database")]
        WorkerDoesNotExistInDatabase(SocketAddr),

        #[error("error occured while trying to rollback state: {0}")]
        RollbackStateError(#[from] RollbackStateError),

        #[error("error occured while trying to assign name to worker: {0}")]
        AssignNameToWorkerError(#[from] AssignNameToWorkerError),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum ReactToChangeOfStateError {
        #[error("database error has occured: {0}")]
        DatabaseError(#[from] sqlx::Error),

        #[error("error occured while trying to set model for worker: {0}")]
        SetModelError(#[from] crate::state::error::SetModelError),

        #[error("worker with socket addr {0} does not exist inside internal database")]
        WorkerDoesNotExistInDatabase(SocketAddr),

        #[error("error occured while trying to generate query: {0}")]
        CreateGenerationQueryError(#[from] CreateGenerationQueryError),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum ReactToNewWorkerPacketError {
        #[error("error occured while trying to react to load command response: {0}")]
        ReactToLoadCommandResponseError(#[from] ReactToLoadCommandResponseError),

        #[error("error occured while trying to react to hello command: {0}")]
        ReactToHelloCommandError(#[from] ReactToHelloCommandError),

        #[error("error occured while trying to get worker data: {0}")]
        GetWorkerError(#[from] GetError),

        #[error("worker with socket addr {0} does not exist inside internal database")]
        WorkerDoesNotExistInDatabase(SocketAddr),

        #[error("database error has occured: {0}")]
        DatabaseError(#[from] sqlx::Error),

        #[error("error occured while trying to rollback state: {0}")]
        RollbackStateError(#[from] RollbackStateError),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum ReactToLoadCommandResponseError {
        #[error("worker with socket addr {0} does not exist inside internal database")]
        WorkerDoesNotExistInDatabase(SocketAddr),

        #[error("database error has occured: {0}")]
        DatabaseError(#[from] sqlx::Error),

        #[error("error occured while trying to get worker data from internal db: {0}")]
        GetError(#[from] GetError),

        #[error("error occured while trying to create a generation query: {0}")]
        CreateGenerationQueryError(#[from] CreateGenerationQueryError),
    }

    #[derive(thiserror::Error, Debug)]
    pub enum RollbackStateError {
        #[error("database error has occured: {0}")]
        DatabaseError(#[from] sqlx::Error),

        #[error("error occured while trying to get worker data from internal db: {0}")]
        GetError(#[from] GetError),

        #[error("error occured while trying to set model: {0}")]
        SetModelError(SetModelError),

        #[error("error occured while trying to set loading model status: {0}")]
        SetLoadingModelStatusError(SetLoadingModelStatusError),

        #[error("error occured while trying to assign name to worker: {0}")]
        AssignNameToWorker(#[from] AssignNameToWorkerError),

        #[error("error occured while trying to create a generation query: {0}")]
        CreateGenerationQueryError(#[from] CreateGenerationQueryError),
    }
}
