use anyhow::Result;
use clap::Parser;
use proto::master::Packet as MasterPacket;
use proto::worker::Packet as WorkerPacket;
use rlpg::tcp::{run_on_socket, RLPGEventBus};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::Level;
use worker::cli::Args;
use worker::handler::{handle_packet, handle_read, handle_write};
use worker::inference::guard::TextModelGuard;
use worker::inference::{Device, TextModelConfig, TextModelFiles};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    let args = Args::parse();

    let device = if let Some(cuda_device) = args.cuda_device {
        Device::CUDA(cuda_device)
    } else {
        Device::CPU
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

    let event_bus = RLPGEventBus::new();
    let stream = TcpStream::connect(format!("{}:{}", args.host, args.port)).await?;
    tracing::info!("Connected to master");
    let addr = stream.peer_addr()?;
    tokio::spawn(run_on_socket(
        stream,
        CancellationToken::new(),
        event_bus.clone(),
        addr,
    ));

    let cancellation_token = CancellationToken::new();
    let cancel = cancellation_token.run_until_cancelled(async {
        let _ = tokio::signal::ctrl_c().await;
    });

    let (itx, irx) = mpsc::channel::<MasterPacket>(32);
    let (otx, orx) = mpsc::channel::<WorkerPacket>(32);
    tokio::spawn(handle_read(
        event_bus.clone(),
        itx,
        cancellation_token.clone(),
    ));
    tokio::spawn(handle_write(event_bus.clone(), orx));
    tokio::spawn(handle_packet(irx, otx, model, config, files));

    cancel.await;
    Ok(())
}
