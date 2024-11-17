use futures::FutureExt;
use tokio_util::sync::CancellationToken;

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

            tracing::info!("{:?}", accepted_token)
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cancellation_token = CancellationToken::new();

    let server_fut = Box::pin(server::start(cancellation_token.clone())).shared();
    let signal_fut = Box::pin(async {
        _ = tokio::signal::ctrl_c().await;
    })
    .shared();

    tokio::select! {
        _ = server_fut.clone() => {},
        _ = signal_fut.clone() => {}
    }

    cancellation_token.cancel();

    if let Err(e) = server_fut.await {
        tracing::error!("{}", e);
    }

    tracing::info!("The tcp listener has been closed.");

    _ = signal_fut.await;

    tracing::info!("The ctrl+c signal listener has been closed.");
}
