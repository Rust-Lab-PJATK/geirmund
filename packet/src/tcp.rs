use std::{future::Future, net::SocketAddr, sync::Arc};
use thiserror::Error;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    sync::broadcast::{error::SendError, Receiver, Sender},
};
use tokio_util::sync::CancellationToken;

use crate::RLPGParser;

pub struct RLPGTcpListener {
    event_bus: RLPGEventBus,
}

#[derive(Error, Debug, Clone)]
pub enum RunServerError {
    #[error("IO error: {0}")]
    IOError(Arc<tokio::io::Error>),

    #[error("Failed to send event through event bus: {0}")]
    SendError(Arc<SendError<RLPGEvent>>),
}

impl From<SendError<RLPGEvent>> for RunServerError {
    fn from(value: SendError<RLPGEvent>) -> Self {
        return RunServerError::SendError(Arc::new(value));
    }
}

impl From<tokio::io::Error> for RunServerError {
    fn from(value: tokio::io::Error) -> Self {
        return RunServerError::IOError(Arc::new(value));
    }
}

impl RLPGTcpListener {
    pub fn new(event_bus: RLPGEventBus) -> Self {
        RLPGTcpListener {
            event_bus: event_bus,
        }
    }

    fn run_on_socket(
        mut socket: TcpStream,
        cancellation_token: CancellationToken,
        socket_event_bus: RLPGEventBus,
    ) -> impl Future<Output = Result<(), RunServerError>> {
        async move {
            let mut parser = RLPGParser::new();
            let mut buf: Vec<u8> = Vec::new();

            loop {
                tokio::select! {
                    _ = cancellation_token.cancelled() => return Ok(()),
                    _ = socket.read_to_end(&mut buf) => {},
                };

                parser.add_to_buffer(&buf);

                match parser.parse() {
                    Ok(Some(parsed_data)) => {
                        socket_event_bus
                            .sender
                            .send(RLPGEvent::NewPacket(parsed_data))?;

                        parser = RLPGParser::new();
                    }
                    Err(_) => return Ok(()),
                    _ => {}
                }
            }
        }
    }

    pub async fn run(
        &self,
        addr: &str,
        cancellation_token: CancellationToken,
    ) -> Result<(), RunServerError> {
        let listener = TcpListener::bind(addr).await?;

        loop {
            let (socket, _) = tokio::select! {
                res = listener.accept() => res?,
                _ = cancellation_token.cancelled() => return Ok(()),
            };

            let cancellation_token = cancellation_token.clone();

            let socket_event_bus = self.event_bus.clone();

            tokio::spawn(Self::run_on_socket(
                socket,
                cancellation_token,
                socket_event_bus,
            ));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RLPGEvent {
    NewPacket(Vec<u8>),

    ClientDisconnected(SocketAddr),
}

#[derive(Debug)]
pub struct RLPGEventBus {
    sender: Sender<RLPGEvent>,
    consumer: Receiver<RLPGEvent>,
}

impl Clone for RLPGEventBus {
    fn clone(&self) -> Self {
        RLPGEventBus {
            sender: self.sender.clone(),
            consumer: self.sender.subscribe(),
        }
    }
}

impl RLPGEventBus {
    pub fn new() -> Self {
        let (sender, consumer) = tokio::sync::broadcast::channel(100);
        RLPGEventBus { sender, consumer }
    }

    pub fn send(
        &mut self,
        value: RLPGEvent,
    ) -> Result<usize, Arc<tokio::sync::broadcast::error::SendError<RLPGEvent>>> {
        self.sender.send(value).map_err(|err| Arc::new(err))
    }

    pub async fn receive(&mut self) -> RLPGEvent {
        loop {
            match self.consumer.recv().await {
                Ok(v) => return v,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::{io::AsyncWriteExt, net::TcpStream};
    use tokio_util::sync::CancellationToken;

    use crate::tcp::RLPGEvent;

    use super::{RLPGEventBus, RLPGTcpListener};

    async fn send_data_to_server(addr: &str, contents: Vec<Vec<u8>>, with_timeout: bool) {
        const VERSION: &str = env!("CARGO_PKG_VERSION");

        let mut connection = TcpStream::connect(addr).await.unwrap();

        let content_length: usize = contents.iter().map(|e| e.len()).sum();

        for (index, content) in contents.iter().enumerate() {
            let mut content = String::from_utf8(content.clone()).unwrap();

            content = if index == 0 {
                format!("RLPG/{VERSION}\n{content_length}\n\n{content}")
            } else {
                content
            };

            connection.write(content.as_bytes()).await.unwrap();

            if with_timeout {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }

    #[tokio::test]
    pub async fn test_tcp_server_works_at_normal_conditions() {
        let mut event_bus = RLPGEventBus::new();
        let server = RLPGTcpListener::new(event_bus.clone());

        let cancellation_token = CancellationToken::new();

        let server_cancel_token = cancellation_token.clone();
        let server_fut =
            tokio::spawn(async move { server.run("127.0.0.1:4340", server_cancel_token).await });

        send_data_to_server("127.0.0.1:4340", vec![b"Hello world".to_vec()], false).await;

        let event = event_bus.receive().await;

        cancellation_token.cancel();

        assert!(matches!(event, RLPGEvent::NewPacket(packet) if b"Hello world".to_vec() == packet));

        let _ = server_fut.await.unwrap();
    }

    #[tokio::test]
    pub async fn test_tcp_server_works_when_sending_data_in_batches() {
        let mut event_bus = RLPGEventBus::new();
        let server = RLPGTcpListener::new(event_bus.clone());

        let cancellation_token = CancellationToken::new();

        let server_cancel_token = cancellation_token.clone();
        let server_fut =
            tokio::spawn(async move { server.run("127.0.0.1:4339", server_cancel_token).await });

        send_data_to_server(
            "127.0.0.1:4339",
            vec![b"Hello ".to_vec(), b"world".to_vec()],
            true,
        )
        .await;

        let event = event_bus.receive().await;

        cancellation_token.cancel();

        assert!(matches!(event, RLPGEvent::NewPacket(packet) if b"Hello world".to_vec() == packet));

        let _ = server_fut.await.unwrap();
    }
}
