use std::{future::Future, net::SocketAddr, sync::Arc};

use proto::{
    master::{ModelType, Packet},
    worker::{GenerateResponse, LoadResponse, WorkerError, WorkerErrorWrapper},
    ConversionError,
};
use rlpg::tcp::{RLPGEvent, RLPGEventBus, RLPGTcpListener, RunServerError};
use thiserror::Error;
use tokio::sync::broadcast::error::SendError;
use tokio_util::sync::CancellationToken;

use prost::{DecodeError, EncodeError, Message};

#[derive(Error, Debug, Clone)]
#[error("failed to convert packet to event")]
struct PacketParsingError;

#[derive(Error, Clone, Debug)]
pub enum WorkerGatewayCommandError {
    #[error("conversion error occured")]
    ConversionError,

    #[error("failed to encode packet for worker: {0}")]
    EncodeError(#[from] EncodeError),

    #[error("failed to decode packet from worker: {0}")]
    DecodeError(#[from] DecodeError),

    #[error("failed to send a message through event bus: {0:?}")]
    EventBusSendError(#[from] Arc<SendError<RLPGEvent>>),

    #[error("worker error occured: {0:?}")]
    WorkerError(WorkerError),
}

impl From<SendError<RLPGEvent>> for WorkerGatewayCommandError {
    fn from(value: SendError<RLPGEvent>) -> Self {
        return WorkerGatewayCommandError::EventBusSendError(Arc::new(value));
    }
}

impl From<ConversionError> for WorkerGatewayCommandError {
    fn from(_: ConversionError) -> Self {
        return WorkerGatewayCommandError::ConversionError;
    }
}

pub struct WorkerGateway;

impl WorkerGateway {
    fn generate_new_request_id() -> u32 {
        rand::random::<u32>()
    }

    fn convert_packet_to_bytes(packet: Packet) -> Result<Vec<u8>, EncodeError> {
        let mut packet_as_bytes: Vec<u8> = Vec::new();
        packet.encode(&mut packet_as_bytes)?;

        Ok(packet_as_bytes)
    }

    async fn get_packet_for_socket_addr(
        rlpg_event_bus: &mut RLPGEventBus,
        socket_addr: &SocketAddr,
    ) -> Result<proto::worker::Packet, DecodeError> {
        loop {
            let packet_payload = match rlpg_event_bus.receive().await {
                RLPGEvent::NewPacketReceived((packet_payload, owner_addr)) => {
                    if owner_addr != *socket_addr {
                        continue;
                    }

                    packet_payload
                }
                _ => continue,
            };

            return proto::worker::Packet::decode(&mut packet_payload.as_slice());
        }
    }

    /// Load the model on the worker.
    ///
    /// First argument is the socket address of the worker.
    /// Second argument is the type of the model that should be loaded.
    pub async fn load_model(
        mut rlpg_event_bus: RLPGEventBus,
        socket_addr: SocketAddr,
        model_type: ModelType,
    ) -> Result<(), WorkerGatewayCommandError> {
        let request_id: u32 = Self::generate_new_request_id();

        let packet = proto::master::Packet::new(proto::master::MasterMessage::LoadCommand(
            proto::master::LoadCommand::new(request_id, model_type),
        ));

        let packet_as_bytes = Self::convert_packet_to_bytes(packet)?;

        rlpg_event_bus.send(RLPGEvent::SendNewPacket((packet_as_bytes, socket_addr)))?;

        loop {
            let packet_payload =
                Self::get_packet_for_socket_addr(&mut rlpg_event_bus, &socket_addr).await?;

            match packet_payload.msg {
                Some(proto::worker::WorkerMessage::LoadResponse(load_response)) => {
                    let load_response: Result<LoadResponse, WorkerErrorWrapper> =
                        load_response.try_into()?;

                    match load_response {
                        Ok(v) => {
                            if v.id != request_id {
                                continue;
                            }

                            return Ok(());
                        }
                        Err(e) => {
                            return Err(WorkerGatewayCommandError::WorkerError(e.try_into()?));
                        }
                    };
                }
                _ => continue,
            };
        }
    }

    pub async fn generate_response(
        mut rlpg_event_bus: RLPGEventBus,
        socket_addr: SocketAddr,
        model_type: ModelType,
        prompt: impl Into<String>,
    ) -> Result<String, WorkerGatewayCommandError> {
        let prompt = prompt.into();

        let request_id = Self::generate_new_request_id();

        let packet = proto::master::Packet::new(proto::master::MasterMessage::GenerateCommand(
            proto::master::GenerateCommand::new(request_id, model_type, prompt),
        ));

        let packet = Self::convert_packet_to_bytes(packet)?;

        rlpg_event_bus.send(RLPGEvent::SendNewPacket((packet, socket_addr)))?;

        loop {
            let packet =
                Self::get_packet_for_socket_addr(&mut rlpg_event_bus, &socket_addr).await?;

            match packet.msg {
                Some(proto::worker::WorkerMessage::GenerateResponse(operation_result)) => {
                    let operation_result: Result<GenerateResponse, WorkerErrorWrapper> =
                        operation_result.try_into()?;

                    match operation_result {
                        Ok(response) => {
                            if response.id != request_id {
                                continue;
                            }

                            return Ok(response.content);
                        }
                        Err(e) => {
                            return Err(WorkerGatewayCommandError::WorkerError(e.try_into()?));
                        }
                    };
                }
                _ => continue,
            };
        }
    }

    pub async fn worker_disconnected(mut rlpg_event_bus: RLPGEventBus, socket_addr: &SocketAddr) {
        loop {
            match rlpg_event_bus.receive().await {
                RLPGEvent::ClientDisconnected((_, addr)) => {
                    if addr == *socket_addr {
                        return;
                    }
                }
                _ => continue,
            }
        }
    }

    pub async fn worker_connected(mut rlpg_event_bus: RLPGEventBus) -> SocketAddr {
        loop {
            match rlpg_event_bus.receive().await {
                RLPGEvent::ClientConnected(addr) => return addr,
                _ => continue,
            };
        }
    }
}

/// Run the server on the given address
///
/// ```
/// let event_bus = EventBus::new();
///
/// tokio::spawn(async move {
///    run("127.0.0.1:4339", CancellationToken::new(), event_bus).await
/// });
/// ```
pub fn run(
    addr: String,
    cancellation_token: CancellationToken,
    rlpg_event_bus: RLPGEventBus,
) -> impl Future<Output = Result<(), RunServerError>> {
    async move {
        let rlpg_server = RLPGTcpListener::new(rlpg_event_bus);

        let addr = addr.clone();
        let server_fut_cancel_token = cancellation_token.clone();

        // server_fut
        tokio::spawn(async move {
            // this weird thing is because of borrow checker complaints
            if let Err(e) = rlpg_server.run(&addr, server_fut_cancel_token).await {
                return Err(e);
            }

            return Ok(());
        });

        cancellation_token.cancelled().await;

        Ok(())
    }
}
