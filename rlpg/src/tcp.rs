use std::{future::Future, net::SocketAddr, sync::Arc};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
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

impl From<Arc<SendError<RLPGEvent>>> for RunServerError {
    fn from(value: Arc<SendError<RLPGEvent>>) -> Self {
        return RunServerError::SendError(value);
    }
}

impl From<tokio::io::Error> for RunServerError {
    fn from(value: tokio::io::Error) -> Self {
        return RunServerError::IOError(Arc::new(value));
    }
}

pub fn run_on_socket(
    mut socket: TcpStream,
    cancellation_token: CancellationToken,
    mut socket_event_bus: RLPGEventBus,
    socket_addr: SocketAddr,
) -> impl Future<Output = Result<(), RunServerError>> {
    async move {
        let mut parser = RLPGParser::new();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => return Ok(()),
                res = socket.read_u8() => {
                    match res {
                        Ok(character) => parser.add_char_to_buffer(character),
                        Err(e) => {
                            let _ = socket.shutdown().await;
                            socket_event_bus.send(RLPGEvent::ClientDisconnected((
                                DisconnectReason::Unknown,
                                socket_addr,
                            )))?;
                            return Err(RunServerError::IOError(Arc::new(e)));
                        }
                    };
                }
            };

            match parser.parse() {
                Ok(Some(parsed_data)) => {
                    socket_event_bus
                        .sender
                        .send(RLPGEvent::NewPacket(parsed_data))?;

                    parser = RLPGParser::new();
                }
                Err(e) => {
                    let _ = socket.shutdown().await;
                    socket_event_bus.send(RLPGEvent::ClientDisconnected((
                        DisconnectReason::InvalidByte,
                        socket_addr,
                    )))?;
                    return Ok(());
                }
                _ => {}
            };
        }
    }
}

impl RLPGTcpListener {
    pub fn new(event_bus: RLPGEventBus) -> Self {
        RLPGTcpListener {
            event_bus: event_bus,
        }
    }

    pub async fn run(
        &self,
        addr: &str,
        cancellation_token: CancellationToken,
    ) -> Result<(), RunServerError> {
        let listener = TcpListener::bind(addr).await?;
        loop {
            let (socket, addr) = tokio::select! {
                res = listener.accept() => res?,
                _ = cancellation_token.cancelled() => return Ok(())
            };

            let cancellation_token = cancellation_token.clone();

            let socket_event_bus = self.event_bus.clone();

            tokio::spawn(run_on_socket(
                socket,
                cancellation_token,
                socket_event_bus,
                addr,
            ));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    InvalidByte,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RLPGEvent {
    NewPacket(Vec<u8>),
    ClientDisconnected((DisconnectReason, SocketAddr)),
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
    use std::time::Duration;

    use tokio::{
        io::{AsyncWriteExt, Interest},
        net::{TcpSocket, TcpStream},
    };
    use tokio_util::sync::CancellationToken;

    use crate::tcp::RLPGEvent;

    use super::{DisconnectReason, RLPGEventBus, RLPGTcpListener};

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

            connection.flush().await.unwrap();

            if index + 1 != content.len() && with_timeout {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
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

        dbg!(event.clone());

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

    #[tokio::test]
    pub async fn test_if_tcp_server_disconnects_a_client_when_client_sends_invalid_payload() {
        let mut event_bus = RLPGEventBus::new();
        let server = RLPGTcpListener::new(event_bus.clone());

        let cancellation_token = CancellationToken::new();

        let server_cancel_token = cancellation_token.clone();
        let server_fut =
            tokio::spawn(async move { server.run("0.0.0.0:4341", server_cancel_token).await });

        let mut client = TcpStream::connect("127.0.0.1:4341").await.unwrap();
        match client.write(b"Some shit").await {
            Ok(v) => assert!(v == 9, "Expected 9 bytes to be sent back."),
            v => assert!(false, "Expected Ok(bytes), received {v:?}"),
        };

        client.flush().await.unwrap();

        let event = event_bus.receive().await;

        assert!(
            matches!(event, RLPGEvent::ClientDisconnected((reason, _)) if reason == DisconnectReason::InvalidByte)
        );

        assert!(
            client.write(b"test").await.is_err(),
            "Expected client to be disconnected."
        );

        cancellation_token.cancel();
        let _ = server_fut.await;
    }
}
