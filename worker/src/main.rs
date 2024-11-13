use anyhow::Result;
use candle_core::backend::BackendDevice;
use candle_core::{CudaDevice, Device};
use clap::Parser;
use proto::master::MasterMessage;
use worker::cli::Args;
use worker::client::Client;
use worker::llama::LLama;
use worker::{TextModel, TextModelConfig, TextModelFiles};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let device = if let Some(cuda_device) = args.cuda_device {
        Device::Cuda(CudaDevice::new(cuda_device)?)
    } else {
        Device::Cpu
    };

    let mut model = LLama::new(
        TextModelConfig::default_builder()
            .with_device(device)
            .build(),
        TextModelFiles::new(args.config, args.tokenizer, args.weights),
    );

    let mut client = Client::connect(format!("{}:{}", args.host, args.port)).await?;

    loop {
        if let Some(packet) = client.connection.read_packet().await? {
            match packet.msg {
                Some(MasterMessage::LoadCommand(_)) => model.load()?,
                Some(MasterMessage::GenerateCommand(generate)) => {
                    println!("Generate prompt: {}", generate.prompt);
                    let answer = model.generate(generate.prompt)?;
                    println!("Generate answer: {}", answer);
                }
                None => {}
            }
        }
    }
}
