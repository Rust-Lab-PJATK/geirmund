use futures::FutureExt;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tui::TuiError;

mod tui;

mod server {
    use thiserror::Error;
    use tokio_util::sync::CancellationToken;

    #[derive(Error, Debug, Clone)]
    pub enum StartServerError {
        #[error("failed to bind the tcp listener to the address {0}; {1}")]
        BindTcpListenerError(String, String),
    }

    pub async fn start(cancellation_token: CancellationToken) -> Result<(), StartServerError> {
        let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
            .await
            .map_err(|err| {
                StartServerError::BindTcpListenerError("0.0.0.0:8080".to_string(), err.to_string())
            })?;

        tracing::info!("The server has been bind on port 8080 for all incoming hosts!");

        while !cancellation_token.is_cancelled() {
            let accepted_token = tokio::select! {
                accepted_socket = listener.accept() => match accepted_socket {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("Failed to accept connection to the listener: {}", e);
                        continue;
                    }
                },
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => continue,
            };
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    // logs are currently going to a file
    let file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open("master.log")
        .unwrap();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(file))
        .init();

    // disable raw mode, because when it is enabled it disables ctrl+c handling
    _ = crossterm::terminal::disable_raw_mode();

    let cancellation_token = CancellationToken::new();

    let server_fut = Box::pin(server::start(cancellation_token.clone())).shared();
    let signal_fut = Box::pin(cancellation_token.run_until_cancelled(async {
        _ = tokio::signal::ctrl_c().await;
    }))
    .shared();
    let tui_fut = Box::pin(tui::Tui::run(cancellation_token.clone())).shared();

    tokio::select! {
        _ = server_fut.clone() => {},
        _ = signal_fut.clone() => {},
        _ = tui_fut.clone() => {},
    }

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
