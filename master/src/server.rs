use std::rc::Rc;

use proto::master::Load;
use thiserror::Error;
use tokio::{io::AsyncWriteExt, sync::broadcast::error::SendError};
use tokio_util::sync::CancellationToken;

use crate::{
    event_bus::{Event, EventBus, ServerEvent, TuiEvent},
    ModelType,
};

#[derive(Error, Debug, Clone)]
pub enum StartServerError {
    #[error("failed to bind the tcp listener to the address {0}; {1}")]
    BindTcpListenerError(String, String),

    #[error("tokio returned RecvError: {0}")]
    TokioReceiveError(tokio::sync::broadcast::error::RecvError),

    #[error("tokio returned SendError: {0}")]
    TokioSendError(Rc<SendError<Event>>),

    #[error("failed to unwrap the rc")]
    RcUnwrapError,

    #[error("io error occured: {0}")]
    IoError(Rc<std::io::Error>),
}

pub async fn start(
    mut event_bus: EventBus,
    cancellation_token: CancellationToken,
) -> Result<(), StartServerError> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .map_err(|err| {
            StartServerError::BindTcpListenerError("0.0.0.0:8080".to_string(), err.to_string())
        })?;

    tracing::info!("The server has been bind on port 8080 for all incoming hosts!");

    while !cancellation_token.is_cancelled() {
        let (mut tcp_stream, _) = tokio::select! {
            accepted_socket = listener.accept() => match accepted_socket {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("Failed to accept connection to the listener: {}", e);
                    continue;
                }
            },
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => continue,
        };

        event_bus
            .send(Event::Server(ServerEvent::ClientConnected))
            .map_err(|err| StartServerError::TokioSendError(Rc::new(err)))?;

        loop {
            let model = match event_bus.receive().await {
                Ok(Event::Tui(TuiEvent::SelectedModel(model))) => model,
                _ => continue,
            };

            tracing::info!("message sent through tcp stream!");

            let model: ModelType = model.into();

            let mut buffer: Vec<u8> = Vec::new();
            proto::master::MasterMessage::LoadCommand(Load::new(model.into())).encode(&mut buffer);

            tracing::info!("message sent through tcp stream!");

            tcp_stream.write(&buffer).await.map_err(|err| {
                tracing::error!("{:?}", err);
                StartServerError::IoError(Rc::new(err))
            })?;

            break;
        }

        // TODO ...
    }

    Ok(())
}

// #[cfg(test)]
// mod tests {
//     #[tokio::test]
//     pub async fn test_the_server() {
//         crate::main();

//         let server = tokio::net::TcpStream::new();
//     }
// }
