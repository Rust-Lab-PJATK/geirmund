use crate::inference::guard::TextModelGuard;
use crate::inference::llama::LLama;
use crate::inference::{TextModel, TextModelConfig, TextModelFiles};
use anyhow::Error as E;
use prost::Message;
use proto::master::{
    GenerateCommand, LoadCommand, MasterMessage, ModelType, Packet as MasterPacket,
};
use proto::worker::{
    GenerateResponse, LoadResponse, Packet as WorkerPacket, WorkerError, WorkerErrorContent,
};
use proto::ProtoResult;
use rlpg::tcp::{RLPGEvent, RLPGEventBus};
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

#[tracing::instrument(skip_all)]
pub async fn handle_write(mut eb: RLPGEventBus, mut orx: Receiver<WorkerPacket>) {
    while let Some(packet) = orx.recv().await {
        match eb.send(RLPGEvent::SendNewPacket(packet.encode_to_vec())) {
            Ok(_) => tracing::debug!("Response sent to master"),
            Err(err) => tracing::error!("Write packet: {}", err),
        }
    }
}

#[tracing::instrument(skip_all)]
pub async fn handle_read(
    mut eb: RLPGEventBus,
    itx: Sender<MasterPacket>,
    cancellation_token: CancellationToken,
) {
    while !cancellation_token.is_cancelled() {
        let reading = async {
            match eb.receive().await {
                RLPGEvent::NewPacketReceived((bytes, _)) => {
                    let packet = MasterPacket::decode(bytes.as_slice())?;
                    itx.send(packet).await?;
                }
                _ => {}
            }
            Ok::<(), E>(())
        };

        match reading.await {
            Ok(_) => tracing::debug!("Received command from master"),
            Err(err) => tracing::error!("Read packet: {}", err),
        }
    }
    tracing::info!("Cancelled")
}

#[tracing::instrument(skip_all)]
pub async fn handle_packet(
    mut irx: Receiver<MasterPacket>,
    otx: Sender<WorkerPacket>,
    model: Arc<TextModelGuard>,
    config: Arc<TextModelConfig>,
    files: Arc<TextModelFiles>,
) {
    while let Some(packet) = irx.recv().await {
        match packet.msg {
            Some(MasterMessage::LoadCommand(load)) => {
                let model = Arc::clone(&model);
                let config = Arc::clone(&config);
                let files = Arc::clone(&files);
                let otx = otx.clone();
                tokio::task::spawn_blocking(move || handle_load(load, model, otx, config, files));
            }
            Some(MasterMessage::GenerateCommand(generate)) => {
                let model = Arc::clone(&model);
                let otx = otx.clone();
                tokio::task::spawn_blocking(move || handle_generate(generate, model, otx));
            }
            None => unreachable!("Received malformed packet"),
        }
    }
}

#[tracing::instrument(skip_all)]
fn handle_load(
    msg: LoadCommand,
    model: Arc<TextModelGuard>,
    otx: Sender<WorkerPacket>,
    config: Arc<TextModelConfig>,
    files: Arc<TextModelFiles>,
) {
    let mut guard = match model.lock_now() {
        Ok(guard) if guard.is_some() => {
            tracing::info!("Model is already loaded");
            let response = WorkerPacket::new_load_response(ProtoResult::Err(
                WorkerError::ModelAlreadyLoaded(WorkerErrorContent::new(msg.id)).into(),
            ));
            blocking_send(&otx, response);
            return;
        }
        Ok(guard) => guard,
        Err(_) => {
            let response = WorkerPacket::new_load_response(ProtoResult::Err(
                WorkerError::ModelBusy(WorkerErrorContent::new(msg.id)).into(),
            ));
            blocking_send(&otx, response);
            return;
        }
    };

    let response = match msg.r#type() {
        ModelType::Llama3v2_1B => match LLama::new(config, files) {
            Ok(llama) => {
                *guard = Some(Box::new(llama));
                tracing::info!("Loaded successfully");
                ProtoResult::Ok(LoadResponse::new(msg.id))
            }
            Err(err) => {
                tracing::error!("Loading: {}", err);
                ProtoResult::Err(WorkerError::LoadingError(WorkerErrorContent::new(msg.id)).into())
            }
        },
    };
    let response = WorkerPacket::new_load_response(response);
    blocking_send(&otx, response);
}

#[tracing::instrument(skip_all)]
fn handle_generate(msg: GenerateCommand, model: Arc<TextModelGuard>, otx: Sender<WorkerPacket>) {
    let response = if let Ok(mut guard) = model.lock_now() {
        match guard.as_mut() {
            Some(g) => match g.generate(msg.prompt) {
                Ok(answer) => {
                    tracing::info!("Generated: {} tokens", answer.len());
                    tracing::info!("Generated: {}", answer);
                    ProtoResult::Ok(GenerateResponse::new(msg.id, answer))
                }
                Err(err) => {
                    tracing::error!("Generation: {}", err);
                    ProtoResult::Err(
                        WorkerError::GenerationError(WorkerErrorContent::new(msg.id)).into(),
                    )
                }
            },
            None => {
                tracing::info!("Model not loaded");
                ProtoResult::Err(
                    WorkerError::ModelNotLoaded(WorkerErrorContent::new(msg.id)).into(),
                )
            }
        }
    } else {
        ProtoResult::Err(WorkerError::ModelBusy(WorkerErrorContent::new(msg.id)).into())
    };

    let response = WorkerPacket::new_generate_response(response);
    blocking_send(&otx, response);
}

#[tracing::instrument(skip_all)]
fn blocking_send<T>(tx: &Sender<T>, msg: T) {
    if let Err(e) = tx.blocking_send(msg) {
        tracing::error!("Sending message: {}", e);
    }
}
