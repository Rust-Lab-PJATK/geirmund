use futures::Stream;
use proto::{MasterPacket, WorkerPacket};
use std::{net::ToSocketAddrs, pin::Pin};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tonic::{transport::Server, Request, Response, Status, Streaming};
use tracing::Level;

pub struct MasterServer {
    cancellation_token: CancellationToken,
    send_request_receiver: mpsc::Receiver<MasterPacket>,
    receive_response_sender: mpsc::Sender<WorkerPacket>,
}

#[tonic::async_trait]
impl proto::master_server::Master for MasterServer {
    // this name is awful, but it's required by tonic.
    // I did not have any choice.
    type StreamStream = Pin<Box<dyn Stream<Item = Result<MasterPacket, Status>> + Send>>;

    async fn stream(
        &self,
        request: Request<Streaming<WorkerPacket>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        let in_stream = request.into_inner();

        let (tx, rx) = mpsc::channel(128);

        let cancellation_token = self.cancellation_token.clone();
        tokio::spawn(async move { while !cancellation_token.is_cancelled() {} });

        let out_stream = ReceiverStream::new(rx);

        Ok(Response::new(Box::pin(out_stream) as Self::StreamStream))
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

    let rlpg_event_bus = RLPGEventBus::new();

    let mut tui = tui::Tui::new(rlpg_event_bus.clone());

    let tui_fut = Box::pin(tui.run(cancellation_token.clone())).shared();
    let server_fut = Box::pin(server::run(
        String::from("127.0.0.1:4339"),
        cancellation_token.clone(),
        rlpg_event_bus,
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
