use anyhow::{Error as E, Result};
use candle_core::backend::BackendDevice;
use candle_core::{CudaDevice, Device};
use clap::Parser;
use proto::master::{Generate, Load, MasterMessage, ModelType, Packet};
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::Level;
use worker::cli::Args;
use worker::client::Client;
use worker::llama::LLama;
use worker::{TextModel, TextModelConfig, TextModelFiles, TextModelGuard};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    let args = Args::parse();

    let device = if let Some(cuda_device) = args.cuda_device {
        Device::Cuda(CudaDevice::new(cuda_device)?)
    } else {
        Device::Cpu
    };

    let config = Arc::new(
        TextModelConfig::default_builder()
            .with_device(device)
            .build(),
    );
    let files = Arc::new(TextModelFiles::new(
        args.config,
        args.tokenizer,
        args.weights,
    ));
    let model = Arc::new(TextModelGuard::empty());
    let client = Arc::new(Mutex::new(
        Client::connect(format!("{}:{}", args.host, args.port)).await?,
    ));

    let cancellation_token = CancellationToken::new();
    let cancel = cancellation_token.run_until_cancelled(async {
        let _ = tokio::signal::ctrl_c().await;
    });

    let (tx, mut rx) = mpsc::channel::<Packet>(32);
    tokio::spawn(handle_read(
        Arc::clone(&client),
        tx,
        cancellation_token.clone(),
    ));
    tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            match packet.msg {
                Some(MasterMessage::LoadCommand(load)) => {
                    let model = Arc::clone(&model);
                    let config = Arc::clone(&config);
                    let files = Arc::clone(&files);
                    tokio::task::spawn_blocking(move || handle_load(load, model, config, files));
                }
                Some(MasterMessage::GenerateCommand(generate)) => {
                    let model = Arc::clone(&model);
                    tokio::task::spawn_blocking(move || handle_generate(generate, model));
                }
                None => unreachable!("Received malformed packet"),
            }
        }
    });

    cancel.await;
    Ok(())
}

#[tracing::instrument(skip_all)]
async fn handle_read(
    client: Arc<Mutex<Client>>,
    tx: Sender<Packet>,
    cancellation_token: CancellationToken,
) {
    while !cancellation_token.is_cancelled() {
        let reading = async {
            if let Some(packet) = client.lock().await.connection.read_packet().await? {
                tx.send(packet).await?;
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
fn handle_load(
    msg: Load,
    model: Arc<TextModelGuard>,
    config: Arc<TextModelConfig>,
    files: Arc<TextModelFiles>,
) {
    match msg.r#type() {
        ModelType::Llama3v2_1B => {
            if let Ok(mut guard) = model.lock_now() {
                if guard.is_some() {
                    tracing::info!("Model is already loaded");
                    return;
                }

                match LLama::new(config, files) {
                    Ok(llama) => {
                        *guard = Some(Box::new(llama));
                        tracing::info!("Loaded successfully");
                    }
                    Err(err) => tracing::error!("Loading: {}", err),
                }
            }
        }
    }
}

#[tracing::instrument(skip_all)]
fn handle_generate(msg: Generate, model: Arc<TextModelGuard>) {
    if let Ok(mut guard) = model.lock_now() {
        match guard.as_mut() {
            Some(g) => match g.generate(msg.prompt) {
                Ok(answer) => tracing::info!("Generated: {} tokens", answer.len()),
                Err(err) => tracing::error!("Generation: {}", err),
            },
            None => tracing::info!("Model not loaded"),
        }
    }
}
