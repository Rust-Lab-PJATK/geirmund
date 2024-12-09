//! # Usage example
//! You run the listener by using [`TcpListenerGateway::run`]. It will start TCP listener on
//! address specified in argument using tokio::spawn under the hood. Returned value is a
//! [`TcpListenerGateway`] that is interconnected with running TCP listener.
//!
//! ```rust
//! let cancellation_token = CancellationToken::new();
//!
//! let tcp_listener_gateway = TcpListenerGateway::run("0.0.0.0:8080",
//! cancellation_token.clone()).await;
//!
//! let tcp_client_gateway = TcpClientGateway::run("0.0.0.0:8080",
//! cancellation_token.clone()).await;
//!
//! let connected_socket_addr = tcp_listener_gateway.listen_for_new_connection().await;
//!
//! tcp_listener_gateway.send_to_client(connected_socket_addr, b"Hello world!".to_vec()).await;
//!
//! cancellation_token.cancel();
//! ```

#[warn(missing_docs)]
use error::tcp_listener::SendToClientError;
use std::future::Future;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::SendError as TokioSendError;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

use crate::{RLPGParser, VERSION};

/// Errors that are used by this module
pub mod error {
    /// Errors for tcp listener
    pub mod tcp_listener {
        use crate::tcp::Event;
        use tokio::sync::broadcast::error::SendError;

        #[derive(Debug)]
        pub enum ReceiveFromClientError {}

        pub enum ReceiveFromAllError {}

        pub enum SendToAllError {}

        #[derive(thiserror::Error, Debug, Clone)]
        pub enum SendToClientError {
            #[error("io error occured while trying to send data to client: {0}")]
            IOError(String),

            #[error("sending data to the client has been too slow, timed out")]
            TimedOut,

            #[error("socket returned an error: {0}")]
            SocketError(String),
        }

        impl From<SendError<Event>> for SendToClientError {
            fn from(value: SendError<Event>) -> Self {
                Self::IOError(format!("{:?}", value.0))
            }
        }

        #[derive(thiserror::Error, Debug, Clone)]
        pub enum RunError {
            #[error("IO Error: {0}")]
            IOError(String),
        }

        impl From<tokio::io::Error> for RunError {
            fn from(value: tokio::io::Error) -> Self {
                Self::IOError(value.to_string())
            }
        }
    }

    /// Errors for tcp client
    pub mod tcp_client {
        pub use super::tcp_listener::SendToClientError as SendError;
        use thiserror::Error;

        pub enum ReceiveError {}

        #[derive(Error, Clone, Debug)]
        pub enum RunError {
            #[error("io error occured: {0}")]
            IOError(String),
        }

        impl From<tokio::io::Error> for RunError {
            fn from(value: tokio::io::Error) -> Self {
                Self::IOError(value.to_string())
            }
        }

        pub enum WaitForConnectError {}

        pub enum WaitForDisconnectError {}
    }
}

/// Tcp listener implementation that uses RLPG protocol.
#[derive(Debug, Clone)]
pub struct TcpListenerGateway {
    event_bus: RLPGEventBus,
}

impl TcpListenerGateway {
    pub async fn run(
        addr: &str,
        cancellation_token: CancellationToken,
    ) -> Result<Self, error::tcp_listener::RunError> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let me = Self {
            event_bus: RLPGEventBus::new(),
        };
        let event_bus = me.event_bus.clone();

        tokio::spawn(async move {
            loop {
                let (socket, addr) = tokio::select! {
                    res = listener.accept() => match res {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::debug!("Error occured while trying to accept new connection to the listener: {e}.");
                            continue
                        }
                    },
                        _ = cancellation_token.cancelled() => return,
                };

                let cancellation_token = cancellation_token.clone();

                let mut socket_event_bus = event_bus.clone();

                if let Err(e) = socket_event_bus.send(Event::SocketToCode(
                    SocketToCodeEvent::ClientConnected(addr),
                )) {
                    tracing::warn!("Error occured while trying to send event through event bus, closing the connection without notice on event bus! ({e})");
                    return;
                }

                tokio::spawn(run_on_socket(socket, cancellation_token, socket_event_bus));
            }
        });

        return Ok(me);
    }

    // TODO: test case for disconnected client before the function started
    // TODO: test case for client that disconnects after function start
    // TODO: normal test case
    pub async fn receive_from_client(
        &mut self,
        socket_addr: &SocketAddr,
    ) -> Result<Vec<u8>, error::tcp_listener::ReceiveFromClientError> {
        // TODO: Check if client is connected
        loop {
            match self.event_bus.receive().await {
                Event::SocketToCode(SocketToCodeEvent::NewPacketReceived {
                    data,
                    socket_addr: ev_socket_addr,
                }) => {
                    if ev_socket_addr != *socket_addr {
                        continue;
                    }

                    return Ok(data);
                }
                _ => {}
            };
        }
    }

    pub async fn receive_from_all(
        &mut self,
    ) -> Result<(Vec<u8>, SocketAddr), error::tcp_listener::ReceiveFromAllError> {
        loop {
            match self.event_bus.receive().await {
                Event::SocketToCode(SocketToCodeEvent::NewPacketReceived { data, socket_addr }) => {
                    return Ok((data, socket_addr))
                }
                _ => {}
            };
        }
    }

    // TODO: test case for disconnected client
    // TODO: test case for timeout with big payload size
    // TODO: test case for timeout
    // TODO: test case for sending itself
    pub async fn send_to_client(
        &mut self,
        client_addr: SocketAddr,
        payload: Vec<u8>,
    ) -> Result<(), error::tcp_listener::SendToClientError> {
        // TODO: Check if client is connected
        return send_new_packet_to_client_event_with_timeout(
            &mut self.event_bus,
            payload,
            &client_addr,
        )
        .await;
    }

    //pub async fn send_to_all(
    //    &self,
    //    payload: Vec<u8>,
    //) -> Result<(), error::tcp_listener::SendToAllError> {
    //    // TODO: finish this func
    //    Ok(())
    //}

    // TODO: test case
    pub async fn listen_for_new_connection(&mut self) -> SocketAddr {
        loop {
            match self.event_bus.receive().await {
                Event::SocketToCode(SocketToCodeEvent::ClientConnected(socket_addr)) => {
                    return socket_addr
                }

                _ => continue,
            }
        }
    }

    // TODO: test case
    pub async fn listen_for_disconnect(&mut self) -> SocketAddr {
        loop {
            match self.event_bus.receive().await {
                Event::SocketToCode(SocketToCodeEvent::ClientDisconnected {
                    id: _,
                    socket_addr,
                }) => return socket_addr,

                _ => continue,
            }
        }
    }
}

/// Tcp client implementation that uses RLPG protocol.
///
/// To connect use [`TcpClientGateway::run`]. The function will return an
/// [`error::tcp_client::RunError::IOError`] if it can't connect to the server
/// **(warn: IO Error can be used for other things as well)**.
///
/// If TcpStream connects successfully, the connection will be converted to a background task using
/// [`tokio::spawn`] under the hood and you will receive an instance of [`TcpClientGateway`] which
/// can be used to control your connection.
///
/// ```rust
/// let client = TcpClientGateway::run("0.0.0.0:8080").await.unwrap();
///
/// client.send(b"Hello world!").await.unwrap();
/// let data = client.receive().await.unwrap();
/// println!("{data:?}");
///
/// client.wait_for_disconnect().await.unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct TcpClientGateway {
    socket_addr: SocketAddr,
    event_bus: RLPGEventBus,
}

impl TcpClientGateway {
    pub async fn wait_for_disconnect(
        &mut self,
    ) -> Result<(), error::tcp_client::WaitForDisconnectError> {
        // TODO: Check if client is connected
        loop {
            match self.event_bus.receive().await {
                Event::SocketToCode(SocketToCodeEvent::ClientDisconnected { id, socket_addr }) => {
                    return Ok(())
                }
                _ => continue,
            }
        }
    }

    /// Send data to the server.
    pub async fn send(&mut self, payload: Vec<u8>) -> Result<(), error::tcp_client::SendError> {
        // TODO: Check if client is connected
        return send_new_packet_to_client_event_with_timeout(
            &mut self.event_bus,
            payload,
            &self.socket_addr,
        )
        .await;
    }

    /// Receive data from the server
    pub async fn receive(&mut self) -> Result<Vec<u8>, error::tcp_client::ReceiveError> {
        // TODO: Check if client is connected

        loop {
            match self.event_bus.receive().await {
                Event::SocketToCode(SocketToCodeEvent::NewPacketReceived { data, socket_addr }) => {
                    return Ok(data);
                }

                _ => {}
            };
        }
    }

    pub async fn run(
        addr: &str,
        cancellation_token: CancellationToken,
    ) -> Result<Self, error::tcp_client::RunError> {
        let tcp_stream = tokio::net::TcpStream::connect(addr).await?;
        let me = Self {
            event_bus: RLPGEventBus::new(),
            socket_addr: tcp_stream.peer_addr()?,
        };
        let event_bus = me.event_bus.clone();

        tokio::spawn(async move { run_on_socket(tcp_stream, cancellation_token, event_bus).await });

        Ok(me)
    }
}

async fn send_new_packet_to_client_event_with_timeout(
    event_bus: &mut RLPGEventBus,
    payload: Vec<u8>,
    client_addr: &SocketAddr,
) -> Result<(), SendToClientError> {
    let minimum_payload_sending_speed = 100; // kilobits/sec
    let bits = payload.len() * 8;

    let timeout_after = minimum_payload_sending_speed * bits; // seconds

    let request_id: u32 = rand::random();

    event_bus.send(Event::CodeToSocket(
        CodeToSocketEvent::SendNewPacketToClient {
            id: request_id,
            data: payload,
            client_addr: *client_addr,
        },
    ))?;

    tokio::select! {
        _ = tokio::time::sleep(std::time::Duration::from_secs(timeout_after as u64)) => {
            return Err(error::tcp_listener::SendToClientError::TimedOut);
        },
        res = wait_for_packet_sent_confirmation_on_event_bus(event_bus, request_id) => res?
    };

    Ok(())
}

async fn wait_for_packet_sent_confirmation_on_event_bus(
    event_bus: &mut RLPGEventBus,
    request_id: u32,
) -> Result<(), SendToClientError> {
    loop {
        match event_bus.receive().await {
            Event::SocketToCode(SocketToCodeEvent::NewPacketSent {
                id: incoming_request_id,
            }) => {
                if incoming_request_id != request_id {
                    continue;
                }

                return Ok(());
            }

            Event::SocketToCode(SocketToCodeEvent::FailedToSendPacket {
                id: incoming_request_id,
                err,
            }) => {
                if incoming_request_id != request_id {
                    continue;
                }

                return Err(SendToClientError::SocketError(err.to_string()));
            }

            _ => {}
        };
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    InvalidByte,
    Unknown,
}

#[derive(Debug, Clone)]
enum Event {
    CodeToSocket(CodeToSocketEvent),
    SocketToCode(SocketToCodeEvent),
}

#[derive(Debug, Clone)]
enum CodeToSocketEvent {
    BroadcastNewPacket {
        id: u32,
        data: Vec<u8>,
    },
    SendNewPacketToClient {
        id: u32,
        data: Vec<u8>,
        client_addr: SocketAddr,
    },
    DisconnectTheClient {
        id: u32,
        socket_addr: SocketAddr,
    },
}

#[derive(Debug, Clone)]
enum SocketToCodeEvent {
    ClientConnected(SocketAddr),
    ClientDisconnected {
        id: Option<u32>, // client can be disconnected on request (CodeToSocketEvent) or unexpectedly
        socket_addr: SocketAddr,
    },
    NewPacketSent {
        id: u32,
    },
    FailedToSendPacket {
        id: u32,
        err: String,
    },
    NewPacketReceived {
        data: Vec<u8>,
        socket_addr: SocketAddr,
    },
}

#[derive(Debug)]
struct RLPGEventBus {
    sender: Sender<Event>,
    receiver: Receiver<Event>,
}

impl Clone for RLPGEventBus {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            receiver: self.sender.subscribe(),
        }
    }
}

impl RLPGEventBus {
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::broadcast::channel(128);
        RLPGEventBus { sender, receiver }
    }

    pub fn send(
        &mut self,
        value: Event,
    ) -> Result<(), tokio::sync::broadcast::error::SendError<Event>> {
        self.sender.send(value)?;
        Ok(())
    }

    pub async fn receive(&mut self) -> Event {
        loop {
            match self.receiver.recv().await {
                Ok(v) => return v,
                _ => {}
            }
        }
    }
}

async fn run_on_socket(
    mut socket: TcpStream,
    cancellation_token: CancellationToken,
    mut socket_event_bus: RLPGEventBus,
) -> impl Future<Output = Result<(), RunOnSocketError>> {
    async move {
        let socket_addr = socket.peer_addr()?; // TODO: handle this error and send disconnect
        let mut parser = RLPGParser::new();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => return Ok(()),
                    res = socket.read_u8() => {
                    match res {
                        Ok(character) => parser.add_char_to_buffer(character),
                        Err(e) => return Err(shutdown_socket_on_io_error(&mut socket, &socket_addr, &mut socket_event_bus, e).await),
                    };
                },
                ev = socket_event_bus.receive() => {
                    match react_to_new_event_on_event_bus(&mut socket, &mut socket_event_bus, &socket_addr, &ev).await {
                        AfterReactingToEvent::GoToNextIteration => continue,
                        AfterReactingToEvent::EndFunction => return Ok(()),
                        AfterReactingToEvent::DoNothing => {},
                    };
                }
            };

            match parser.parse() {
                Ok(Some(parsed_data)) => {
                    if let Err(e) = socket_event_bus.send(Event::SocketToCode(
                        SocketToCodeEvent::NewPacketReceived {
                            data: parsed_data,
                            socket_addr: socket_addr.clone(),
                        },
                    )) {
                        shutdown_socket_with_optional_warn(&mut socket, &socket_addr, &mut socket_event_bus, format!("[RLPG] Error occured while trying to send NewPacketReceived event: {e}").into()).await;
                        return Ok(());
                    }

                    parser = RLPGParser::new();
                }
                Err(e) => {
                    shutdown_socket_with_optional_warn(
                        &mut socket,
                        &socket_addr,
                        &mut socket_event_bus,
                        format!(
                            "Client sent invalid byte, disconnecting him: {socket_addr}. ({e})"
                        )
                        .into(),
                    )
                    .await;
                    return Ok(());
                }
                _ => {}
            };
        }
    }
}

#[derive(thiserror::Error, Debug, Clone)]
enum RunOnSocketError {
    #[error("IO error: {0}")]
    IOError(String),

    #[error("Failed to send event through event bus: {0}")]
    SendError(String),
}

impl From<TokioSendError<Event>> for RunOnSocketError {
    fn from(value: TokioSendError<Event>) -> Self {
        return RunOnSocketError::SendError(value.to_string());
    }
}

impl From<tokio::io::Error> for RunOnSocketError {
    fn from(value: tokio::io::Error) -> Self {
        return RunOnSocketError::IOError(value.to_string());
    }
}

enum AfterReactingToEvent {
    GoToNextIteration,
    EndFunction,
    DoNothing,
}

async fn react_to_new_event_on_event_bus(
    socket: &mut TcpStream,
    socket_event_bus: &mut RLPGEventBus,
    socket_addr: &SocketAddr,

    event: &Event,
) -> AfterReactingToEvent {
    match event {
        Event::CodeToSocket(code_to_socket_ev) => {
            match code_to_socket_ev {
                CodeToSocketEvent::DisconnectTheClient {
                    id,
                    socket_addr: event_target_socket_addr,
                } => {
                    if *event_target_socket_addr != *socket_addr {
                        return AfterReactingToEvent::DoNothing;
                    }

                    // result is purposefully ignored; even if the socket is shut down incorrectly, the
                    // client will be disconnected anyway
                    let _ = socket.shutdown().await;

                    if let Err(e) = socket_event_bus.send(Event::SocketToCode(
                        SocketToCodeEvent::ClientDisconnected {
                            id: Some(*id),
                            socket_addr: *socket_addr,
                        },
                    )) {
                        tracing::debug!(
                        "[RLPG] Error occured while trying to send ClientDisconnected event, normally this would be a warn, but we are disconnecting the client anyway. ({e})"
                    );
                    }

                    AfterReactingToEvent::EndFunction
                }

                CodeToSocketEvent::SendNewPacketToClient {
                    id,
                    data,
                    client_addr: event_target_socket_addr,
                } => {
                    if *event_target_socket_addr != *socket_addr {
                        return AfterReactingToEvent::DoNothing;
                    }

                    let content_length = data.len();
                    let header = format!("RLPG/{VERSION}\n{content_length}\n\n");
                    if let Err(err) = socket.write_all(&header.as_bytes()).await {
                        tracing::warn!(
                            "[RLPG] Failed to write to socket {socket_addr}, closing. ({err})"
                        );

                        shutdown_socket_with_optional_warn(
                            socket,
                            socket_addr,
                            socket_event_bus,
                            Some(format!("[RLPG] Error occured while trying to send event through event bus: {err}"))
                        ).await;

                        return AfterReactingToEvent::EndFunction;
                    }

                    if let Err(e) = socket.write_all(data).await {
                        shutdown_socket_with_optional_warn(
                            socket,
                            socket_addr,
                            socket_event_bus,
                            Some(format!("[RLPG] Error occured while trying to write data to the socket, disconnecting the client {socket_addr}. ({e})")),
                        )
                        .await;

                        return AfterReactingToEvent::EndFunction;
                    }

                    if let Err(e) = socket_event_bus.send(Event::SocketToCode(
                        SocketToCodeEvent::NewPacketSent { id: *id },
                    )) {
                        shutdown_socket_with_optional_warn(
                            socket,
                            socket_addr,
                            socket_event_bus,
                            Some(format!(
                                "[RLPG] failed to send NewPacketSent event on event bus: {e}"
                            )),
                        )
                        .await;

                        return AfterReactingToEvent::EndFunction;
                    }

                    AfterReactingToEvent::GoToNextIteration
                }

                _ => AfterReactingToEvent::DoNothing,
            }
        }
        _ => AfterReactingToEvent::DoNothing,
    }
}

async fn shutdown_socket_with_optional_warn(
    socket: &mut TcpStream,
    socket_addr: &SocketAddr,
    socket_event_bus: &mut RLPGEventBus,
    message: Option<String>,
) {
    let _ = socket.shutdown().await;
    let _ = socket_event_bus.send(Event::SocketToCode(SocketToCodeEvent::ClientDisconnected {
        id: None,
        socket_addr: *socket_addr,
    }));
    if let Some(message) = message {
        tracing::warn!(message);
    }
}

async fn shutdown_socket_on_io_error(
    socket: &mut TcpStream,
    socket_addr: &SocketAddr,
    socket_event_bus: &mut RLPGEventBus,
    error: tokio::io::Error,
) -> RunOnSocketError {
    shutdown_socket_with_optional_warn(socket, socket_addr, socket_event_bus, None).await;
    return RunOnSocketError::IOError(error.to_string());
}
