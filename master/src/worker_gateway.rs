use proto::{
    master::{GenerateCommand, LoadCommand, ModelType, Packet as MasterPacket},
    worker::{Packet as WorkerPacket, WorkerError, WorkerMessage},
    ConversionError,
};
use rlpg::tcp::{RLPGEvent, RLPGEventBus, RLPGTcpListener, RunServerError};
use std::{net::SocketAddr, sync::Arc};
use thiserror::Error;
use tokio::sync::broadcast::error::SendError;
use tokio_util::sync::CancellationToken;

use prost::{DecodeError, EncodeError, Message};
use tokio::sync::RwLock;

pub struct WorkerGateway {
    pub event_bus: RLPGEventBus,
    pub workers: Arc<RwLock<Vec<SocketAddr>>>,
}

#[allow(dead_code)]
impl WorkerGateway {
    pub fn new(event_bus: RLPGEventBus) -> Self {
        Self {
            event_bus,
            workers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Load the model on the worker.
    ///
    /// First argument is the socket address of the worker.
    /// Second argument is the type of the model that should be loaded.
    pub async fn load_model(
        &self,
        model_type: ModelType,
        addr: SocketAddr,
    ) -> Result<(), WorkerGatewayCommandError> {
        let request_id: u32 = Self::generate_new_request_id();

        let packet = MasterPacket::new_load_command(LoadCommand::new(request_id, model_type));

        let mut eb = self.event_bus.clone();
        eb.send(RLPGEvent::SendNewPacketToParticularClient((
            packet.encode_to_vec(),
            addr,
        )))?;

        loop {
            let packet_payload = Self::recv_packet_from_addr(&mut eb, &addr).await?;

            match packet_payload.msg {
                Some(WorkerMessage::LoadResponse(load_response)) => {
                    match load_response.try_into()? {
                        Ok(v) if v.id != request_id => continue,
                        Ok(_) => return Ok(()),
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
        &self,
        model_type: ModelType,
        prompt: impl Into<String>,
    ) -> Result<String, WorkerGatewayCommandError> {
        let prompt = prompt.into();

        let request_id = Self::generate_new_request_id();

        let packet = MasterPacket::new_generate_command(GenerateCommand::new(
            request_id, model_type, prompt,
        ));

        // Strategy - first worker
        let mut eb = self.event_bus.to_owned();
        let workers = Arc::clone(&self.workers);
        let worker = Self::first_worker_strategy(workers.read().await.as_ref())?.clone();

        eb.send(RLPGEvent::SendNewPacketToParticularClient((
            packet.encode_to_vec(),
            worker,
        )))?;

        loop {
            let packet = Self::recv_packet_from_addr(&mut eb, &worker).await?;

            match packet.msg {
                Some(WorkerMessage::GenerateResponse(generate_response)) => {
                    match generate_response.try_into()? {
                        Ok(response) if response.id != request_id => continue,
                        Ok(response) => return Ok(response.content),
                        Err(e) => {
                            return Err(WorkerGatewayCommandError::WorkerError(e.try_into()?));
                        }
                    };
                }
                _ => continue,
            };
        }
    }

    pub async fn worker_disconnected(&self) -> SocketAddr {
        loop {
            if let RLPGEvent::ClientDisconnected((_, addr)) = self.event_bus.clone().receive().await
            {
                return addr;
            }
        }
    }

    pub async fn worker_connected(&self) -> SocketAddr {
        loop {
            if let RLPGEvent::ClientConnected(addr) = self.event_bus.clone().receive().await {
                return addr;
            }
        }
    }

    fn generate_new_request_id() -> u32 {
        rand::random::<u32>()
    }

    async fn recv_packet_from_addr(
        rlpg_event_bus: &mut RLPGEventBus,
        socket_addr: &SocketAddr,
    ) -> Result<WorkerPacket, DecodeError> {
        loop {
            let packet_payload = match rlpg_event_bus.receive().await {
                RLPGEvent::NewPacketReceived((_, owner_addr)) if owner_addr != *socket_addr => {
                    continue
                }
                RLPGEvent::NewPacketReceived((payload, _)) => payload,
                _ => continue,
            };

            return WorkerPacket::decode(&mut packet_payload.as_slice());
        }
    }

    fn first_worker_strategy(
        workers: &Vec<SocketAddr>,
    ) -> Result<&SocketAddr, WorkerGatewayCommandError> {
        workers
            .first()
            .ok_or(WorkerGatewayCommandError::NoAvailableWorkers)
    }
}

#[derive(Error, Clone, Debug)]
pub enum WorkerGatewayCommandError {
    #[error("conversion error occurred")]
    ConversionError,

    #[error("failed to encode packet for worker: {0}")]
    EncodeError(#[from] EncodeError),

    #[error("failed to decode packet from worker: {0}")]
    DecodeError(#[from] DecodeError),

    #[error("failed to send a message through event bus: {0:?}")]
    EventBusSendError(#[from] Arc<SendError<RLPGEvent>>),

    #[error("no available workers currently")]
    NoAvailableWorkers,

    #[error("worker error occurred: {0:?}")]
    WorkerError(WorkerError),
}

impl From<SendError<RLPGEvent>> for WorkerGatewayCommandError {
    fn from(value: SendError<RLPGEvent>) -> Self {
        WorkerGatewayCommandError::EventBusSendError(Arc::new(value))
    }
}

impl From<ConversionError> for WorkerGatewayCommandError {
    fn from(_: ConversionError) -> Self {
        WorkerGatewayCommandError::ConversionError
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
pub async fn run_tcp(
    port: usize,
    cancellation_token: CancellationToken,
    rlpg_event_bus: RLPGEventBus,
) -> Result<(), RunServerError> {
    let rlpg_server = RLPGTcpListener::new(rlpg_event_bus);
    let server_fut_cancel_token = cancellation_token.clone();

    // server_fut
    tokio::spawn(async move {
        rlpg_server
            .run(&format!("0.0.0.0:{}", port), server_fut_cancel_token)
            .await
    });
    cancellation_token.cancelled().await;

    Ok(())
}

pub async fn run_workers_watcher(worker_gateway: Arc<WorkerGateway>, cancel: CancellationToken) {
    while !cancel.is_cancelled() {
        tokio::select! {
            worker = worker_gateway.worker_connected() => {
                let _ = worker_gateway.load_model(ModelType::Llama3v2_1B, worker).await;
                worker_gateway.workers.write().await.push(worker);
            },
            worker = worker_gateway.worker_disconnected() => {
                worker_gateway.workers.write().await.retain(|w| *w != worker);
            }
        }
    }
}
