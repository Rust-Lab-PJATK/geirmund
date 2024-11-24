use anyhow::Result;
use candle_core::backend::BackendDevice;
use candle_core::{CudaDevice, Device};
use clap::Parser;
use proto::master::Packet as MasterPacket;
use proto::worker::Packet as WorkerPacket;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::Level;
use worker::cli::Args;
use worker::handler::{handle_packet, handle_read, handle_write};
use worker::inference::guard::TextModelGuard;
use worker::inference::{TextModelConfig, TextModelFiles};
use worker::tcp::Client;

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
    let client = Client::connect(format!("{}:{}", args.host, args.port)).await?;

    let cancellation_token = CancellationToken::new();
    let cancel = cancellation_token.run_until_cancelled(async {
        let _ = tokio::signal::ctrl_c().await;
    });

    let (itx, irx) = mpsc::channel::<MasterPacket>(32);
    let (otx, orx) = mpsc::channel::<WorkerPacket>(32);
    tokio::spawn(handle_read(client.reader, itx, cancellation_token.clone()));
    tokio::spawn(handle_write(client.writer, orx));
    tokio::spawn(handle_packet(irx, otx, model, config, files));

    cancel.await;
    Ok(())
}
