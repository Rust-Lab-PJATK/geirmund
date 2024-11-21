use std::fmt::Display;

use event_bus::EventBus;
use futures::FutureExt;
use tokio_util::sync::CancellationToken;
use tracing::Level;
use tui::TuiError;

mod event_bus;
mod server;
mod tui;

#[derive(Debug, Clone, Copy)]
pub enum ModelType {
    Llama3v2_1B,
}

impl From<proto::master::ModelType> for ModelType {
    fn from(value: proto::master::ModelType) -> Self {
        match value {
            proto::master::ModelType::Llama3v2_1B => Self::Llama3v2_1B,
        }
    }
}

impl Into<proto::master::ModelType> for ModelType {
    fn into(self) -> proto::master::ModelType {
        match self {
            Self::Llama3v2_1B => proto::master::ModelType::Llama3v2_1B,
        }
    }
}

impl Display for ModelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Llama3v2_1B => write!(f, "Llama v3.2 1B"),
        }
    }
}

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "debug");

    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_level(true)
        .with_file(true)
        .with_line_number(true)
        .with_max_level(Level::DEBUG) // TODO by env
        .with_writer(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("master.log")
                .unwrap(),
        )
        .finish();

    tracing::subscriber::set_global_default(subscriber).unwrap();

    // disable raw mode, because when it is enabled it disables ctrl+c handling
    _ = crossterm::terminal::disable_raw_mode();

    let cancellation_token = CancellationToken::new();

    let mut event_bus = EventBus::new();

    let mut tui = tui::Tui::new(event_bus.clone_mut());

    let tui_fut = Box::pin(tui.run(cancellation_token.clone())).shared();
    let server_fut = Box::pin(server::start(
        event_bus.clone_mut(),
        cancellation_token.clone(),
    ))
    .shared();
    let signal_fut = Box::pin(cancellation_token.run_until_cancelled(async {
        _ = tokio::signal::ctrl_c().await;
    }))
    .shared();

    tokio::select! {
        _ = server_fut.clone() => {},
        _ = signal_fut.clone() => {},
        _ = tui_fut.clone() => {},
    };

    cancellation_token.cancel();

    if let Err(e) = tui_fut.await {
        if e != TuiError::Cancelled {
            tracing::error!("{}", e);
            eprintln!("Error occured within TUI: {e}");
        }
    }

    if let Err(e) = server_fut.await {
        tracing::error!("{}", e);
        eprintln!("Error occured within tcp server: {e}");
    }

    tracing::info!("The tcp listener has been closed.");

    _ = signal_fut.await;

    tracing::info!("The ctrl+c signal listener has been closed.");
}
