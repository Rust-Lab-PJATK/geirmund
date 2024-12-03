use crate::cli::Args;
use crate::client_gateway::ClientGateway;
use clap::Parser;
use futures::FutureExt;
use rlpg::tcp::RLPGEventBus;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::Level;
use worker_gateway::WorkerGateway;

mod cli;
mod client_gateway;
mod worker_gateway;

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "debug");
    let args = Args::parse();
    setup_tracing(&args.verbose, &args.logfile);

    tracing::info!(
        "Starting master\n{}",
        include_str!("assets/master_ascii_art.txt")
    );
    let cancellation_token = CancellationToken::new();
    let rlpg_event_bus = RLPGEventBus::new();
    let worker_gateway = Arc::new(WorkerGateway::new(rlpg_event_bus.clone()));

    let tcp_server_fut = Box::pin(worker_gateway::run_tcp(
        args.workers_port,
        cancellation_token.clone(),
        rlpg_event_bus.clone(),
    ))
    .shared();

    let workers_watcher_fut = Box::pin(worker_gateway::run_workers_watcher(
        Arc::clone(&worker_gateway),
        cancellation_token.clone(),
    ))
    .shared();

    let client_gateway = ClientGateway::new(
        args.client_port,
        Arc::clone(&worker_gateway),
        cancellation_token.clone(),
    );
    let http_server_fut = Box::pin(client_gateway.start()).shared();

    let signal_fut = Box::pin(cancellation_token.run_until_cancelled(async {
        let _ = tokio::signal::ctrl_c().await;
    }))
    .shared();

    tokio::select! {
        _ = tcp_server_fut.clone() => {},
        _ = workers_watcher_fut.clone() => {},
        _ = http_server_fut.clone() => {},
        _ = signal_fut.clone() => {},
    }
    cancellation_token.cancel();

    if let Err(e) = tcp_server_fut.await {
        tracing::error!("{}", e);
        eprintln!("Error occurred within tcp server: {e}");
    }

    if let Err(e) = http_server_fut.await {
        tracing::error!("{}", e);
        eprintln!("Error occurred within http server: {e}");
    }

    signal_fut.await;
    tracing::info!("The ctrl+c signal listener has been closed.");
}

fn setup_tracing(verbose: &bool, logfile: &Option<String>) {
    let logging_level = if *verbose { Level::DEBUG } else { Level::INFO };

    let tracing_conf = tracing_subscriber::fmt()
        .compact()
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .with_max_level(logging_level);

    if let Some(file) = logfile {
        tracing_conf
            .with_writer(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(file)
                    .expect("Couldn't open log file"),
            )
            .init();
    } else {
        tracing_conf.with_writer(std::io::stdout).init();
    }
}
