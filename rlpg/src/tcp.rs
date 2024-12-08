use error::tcp_listener::SendToClientError;
use std::future::Future;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::SendError as TokioSendError;
use tokio::sync::broadcast::{Receiver, Sender};
use tokio_util::sync::CancellationToken;

use crate::{RLPGParser, VERSION};

pub mod error {
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

    pub mod tcp_client {
        pub enum SendError {}

        pub enum ReceiveError {}

        pub enum RunError {}

        pub enum WaitForConnectError {}

        pub enum WaitForDisconnectError {}
    }
}

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

    pub async fn receive_from_client(
        &mut self,
        socket_addr: &SocketAddr,
    ) -> Result<Vec<u8>, error::tcp_listener::ReceiveFromClientError> {
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

    // TODO: test case for timeout with big payload size
    // TODO: test case for timeout
    // TODO: test case for sending itself
    pub async fn send_to_client(
        &mut self,
        client_addr: SocketAddr,
        payload: Vec<u8>,
    ) -> Result<(), error::tcp_listener::SendToClientError> {
        let minimum_payload_sending_speed = 100; // kilobits/sec
        let bits = payload.len() * 8;

        let timeout_after = minimum_payload_sending_speed * bits; // seconds

        let request_id: u32 = rand::random();

        self.event_bus.send(Event::CodeToSocket(
            CodeToSocketEvent::SendNewPacketToClient {
                id: request_id,
                data: payload,
                client_addr,
            },
        ))?;

        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(timeout_after as u64)) => {
                return Err(error::tcp_listener::SendToClientError::TimedOut);
            },
            res = self.wait_for_packet_sent_confirmation(request_id) => res?
        };

        Ok(())
    }

    async fn wait_for_packet_sent_confirmation(
        &mut self,
        request_id: u32,
    ) -> Result<(), SendToClientError> {
        loop {
            match self.event_bus.receive().await {
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

#[derive(Debug, Clone)]
pub struct TcpClient {}

impl TcpClient {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn wait_for_disconnect(
        &self,
    ) -> Result<(), error::tcp_client::WaitForDisconnectError> {
        Ok(())
    }

    pub async fn wait_for_connect(&self) -> Result<(), error::tcp_client::WaitForConnectError> {
        Ok(())
    }

    pub async fn send(&self, payload: Vec<u8>) -> Result<(), error::tcp_client::SendError> {
        Ok(())
    }

    pub async fn receive(&self) -> Result<Vec<u8>, error::tcp_client::ReceiveError> {
        Ok(Vec::new())
    }

    pub fn run(
        &self,
        cancellation_token: CancellationToken,
    ) -> impl Future<Output = Result<(), error::tcp_client::RunError>> {
        async move { Ok(()) }
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
        Event::CodeToSocket(code_to_socket_ev) => match code_to_socket_ev {
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

                AfterReactingToEvent::GoToNextIteration
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

                    let _ = socket.shutdown().await;

                    if let Err(err) = socket_event_bus.send(Event::SocketToCode(
                        SocketToCodeEvent::ClientDisconnected {
                            id: None,
                            socket_addr: *socket_addr,
                        },
                    )) {
                        tracing::warn!("[RLPG] Error occured while trying to send event through event bus: {err}");
                    }

                    return AfterReactingToEvent::EndFunction;
                }

                if let Err(e) = socket.write_all(data).await {
                    let _ = socket.shutdown().await;
                    let _ = socket_event_bus.send(Event::SocketToCode(
                        SocketToCodeEvent::ClientDisconnected {
                            id: None,
                            socket_addr: *socket_addr,
                        },
                    ));

                    return AfterReactingToEvent::EndFunction;
                }

                socket_event_bus.send(Event::SocketToCode(SocketToCodeEvent::ClientDisconnected {
                    id: Some(*id),
                    socket_addr: *event_target_socket_addr,
                }));

                AfterReactingToEvent::GoToNextIteration
            }

            _ => AfterReactingToEvent::DoNothing,
        },
        _ => AfterReactingToEvent::DoNothing,
    }
}

async fn shutdown_socket_on_io_error(
    socket: &mut TcpStream,
    socket_addr: &SocketAddr,
    socket_event_bus: &mut RLPGEventBus,
    error: tokio::io::Error,
) -> RunOnSocketError {
    let _ = socket.shutdown().await;
    socket_event_bus.send(Event::SocketToCode(SocketToCodeEvent::ClientDisconnected {
        id: None,
        socket_addr: *socket_addr,
    }));
    return RunOnSocketError::IOError(error.to_string());
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
                    socket_event_bus.send(Event::SocketToCode(
                        SocketToCodeEvent::NewPacketReceived {
                            data: parsed_data,
                            socket_addr: socket_addr.clone(),
                        },
                    ));

                    parser = RLPGParser::new();
                }
                Err(e) => {
                    let _ = socket.shutdown().await;
                    socket_event_bus.send(Event::SocketToCode(
                        SocketToCodeEvent::ClientDisconnected {
                            id: None,
                            socket_addr: socket_addr.clone(),
                        },
                    ));
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

//pub struct RLPGTcpListener {
//    event_bus: RLPGEventBus,
//}
//
//impl RLPGTcpListener {
//    pub fn new(event_bus: RLPGEventBus) -> Self {
//        RLPGTcpListener {
//            event_bus: event_bus,
//        }
//    }
//
//    pub async fn run(
//        &self,
//        addr: &str,
//        cancellation_token: CancellationToken,
//    ) -> Result<(), RunServerError> {
//    }
//}
//
//#[cfg(test)]
//mod tests {
//    use std::time::Duration;
//
//    use tokio::{
//        io::{AsyncReadExt, AsyncWriteExt, Interest},
//        net::{TcpSocket, TcpStream},
//    };
//    use tokio_util::sync::CancellationToken;
//
//    use crate::{tcp::RLPGEvent, VERSION};
//
//    use super::{DisconnectReason, RLPGEventBus, RLPGTcpListener};
//
//    async fn send_data_to_server(addr: &str, contents: Vec<Vec<u8>>, with_timeout: bool) {
//        const VERSION: &str = env!("CARGO_PKG_VERSION");
//
//        let mut connection = TcpStream::connect(addr).await.unwrap();
//
//        let content_length: usize = contents.iter().map(|e| e.len()).sum();
//
//        for (index, content) in contents.iter().enumerate() {
//            let mut content = String::from_utf8(content.clone()).unwrap();
//
//            content = if index == 0 {
//                format!("RLPG/{VERSION}\n{content_length}\n\n{content}")
//            } else {
//                content
//            };
//
//            connection.write(content.as_bytes()).await.unwrap();
//
//            connection.flush().await.unwrap();
//
//            if index + 1 != content.len() && with_timeout {
//                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
//            }
//        }
//    }
//
//    #[tokio::test]
//    pub async fn test_tcp_server_works_at_normal_conditions() {
//        let mut event_bus = RLPGEventBus::new();
//        let server = RLPGTcpListener::new(event_bus.clone());
//
//        let cancellation_token = CancellationToken::new();
//
//        let server_cancel_token = cancellation_token.clone();
//        let server_fut =
//            tokio::spawn(async move { server.run("127.0.0.1:4340", server_cancel_token).await });
//
//        send_data_to_server("127.0.0.1:4340", vec![b"Hello world".to_vec()], false).await;
//
//        let event = event_bus.receive().await;
//        assert!(matches!(event, RLPGEvent::ClientConnected(_)));
//
//        let event = event_bus.receive().await;
//
//        cancellation_token.cancel();
//
//        assert!(
//            matches!(event, RLPGEvent::NewPacketReceived(packet) if b"Hello world".to_vec() == packet.0)
//        );
//
//        let _ = server_fut.await.unwrap();
//    }
//
//    #[tokio::test]
//    pub async fn test_tcp_server_works_when_sending_data_in_batches() {
//        let mut event_bus = RLPGEventBus::new();
//        let server = RLPGTcpListener::new(event_bus.clone());
//
//        let cancellation_token = CancellationToken::new();
//
//        let server_cancel_token = cancellation_token.clone();
//        let server_fut =
//            tokio::spawn(async move { server.run("127.0.0.1:4339", server_cancel_token).await });
//
//        send_data_to_server(
//            "127.0.0.1:4339",
//            vec![b"Hello ".to_vec(), b"world".to_vec()],
//            true,
//        )
//        .await;
//
//        let event = event_bus.receive().await;
//        assert!(matches!(event, RLPGEvent::ClientConnected(_)));
//
//        let event = event_bus.receive().await;
//
//        cancellation_token.cancel();
//
//        assert!(
//            matches!(event, RLPGEvent::NewPacketReceived(packet) if b"Hello world".to_vec() == packet.0)
//        );
//
//        let _ = server_fut.await.unwrap();
//    }
//
//    #[tokio::test]
//    pub async fn test_if_tcp_server_disconnects_a_client_when_client_sends_invalid_payload() {
//        let mut event_bus = RLPGEventBus::new();
//        let server = RLPGTcpListener::new(event_bus.clone());
//
//        let cancellation_token = CancellationToken::new();
//
//        let server_cancel_token = cancellation_token.clone();
//        let server_fut =
//            tokio::spawn(async move { server.run("0.0.0.0:4341", server_cancel_token).await });
//
//        let mut client = TcpStream::connect("127.0.0.1:4341").await.unwrap();
//        match client.write(b"Some shit").await {
//            Ok(v) => assert!(v == 9, "Expected 9 bytes to be sent back."),
//            v => assert!(false, "Expected Ok(bytes), received {v:?}"),
//        };
//
//        client.flush().await.unwrap();
//
//        let event = event_bus.receive().await;
//        assert!(matches!(event, RLPGEvent::ClientConnected(_)));
//
//        let event = event_bus.receive().await;
//
//        assert!(
//            matches!(event, RLPGEvent::ClientDisconnected((reason, _)) if reason == DisconnectReason::InvalidByte)
//        );
//
//        assert!(
//            client.write(b"test").await.is_err(),
//            "Expected client to be disconnected."
//        );
//
//        cancellation_token.cancel();
//        let _ = server_fut.await;
//    }
//
//    #[tokio::test]
//    pub async fn test_tcp_server_sends_valid_payload_on_send_new_packet_event() {
//        let mut event_bus = RLPGEventBus::new();
//        let server = RLPGTcpListener::new(event_bus.clone());
//
//        let cancellation_token = CancellationToken::new();
//
//        let server_cancel_token = cancellation_token.clone();
//        let server_fut =
//            tokio::spawn(async move { server.run("0.0.0.0:4342", server_cancel_token).await });
//
//        let mut client = TcpStream::connect("127.0.0.1:4342").await.unwrap();
//
//        let event = event_bus.receive().await;
//        assert!(matches!(event, RLPGEvent::ClientConnected(_)));
//
//        let socket_addr = match event {
//            RLPGEvent::ClientConnected(socket_addr) => socket_addr,
//            _ => panic!("Expected ClientConnected event."),
//        };
//
//        event_bus
//            .send(RLPGEvent::SendNewPacketToParticularClient((
//                "Hello world!".as_bytes().to_vec(),
//                socket_addr,
//            )))
//            .unwrap();
//
//        let correct_payload = format!("RLPG/{}\n12\n\nHello world!", VERSION);
//
//        let mut buf: Vec<u8> = Vec::new();
//
//        while buf.len() < correct_payload.len() {
//            let byte = client.read_u8().await.unwrap();
//            buf.push(byte);
//        }
//
//        let read = client.try_read(&mut buf).is_err();
//
//        cancellation_token.cancel();
//
//        assert!(String::from_utf8(buf).unwrap() == correct_payload);
//        assert!(read);
//    }
//}

#[cfg(test)]
mod tests {
    use std::{
        net::Ipv4Addr,
        sync::{Arc, LazyLock},
    };

    use crate::tcp::*;
    use rand::random;
    use tokio::{
        net::TcpListener,
        sync::{oneshot, Mutex},
    };

    static LAST_SOCKET_PORT: LazyLock<Arc<Mutex<u16>>> =
        LazyLock::new(|| Arc::new(Mutex::new(4038)));

    async fn get_next_socket_port() -> u16 {
        let value_under_lazy_lock = LAST_SOCKET_PORT.clone();
        let ptr_to_value_under_mutex = &mut *value_under_lazy_lock.lock().await;
        *ptr_to_value_under_mutex = *ptr_to_value_under_mutex + 1;
        return *ptr_to_value_under_mutex;
    }

    fn random_socket_addr() -> SocketAddr {
        SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::new(
                random::<u8>() % 254,
                random::<u8>() % 254,
                random::<u8>() % 254,
                random::<u8>() % 254,
            )),
            random(),
        )
    }

    #[tokio::test]
    pub async fn test_if_tcp_listener_receives_from_particular_client_when_simulated() {
        let cancellation_token = CancellationToken::new();
        let mut tcp_listener = TcpListenerGateway::run(
            &format!("0.0.0.0:{}", get_next_socket_port().await),
            cancellation_token.clone(),
        )
        .await
        .unwrap();
        let mut event_bus = tcp_listener.event_bus.clone();

        let socket_addr = random_socket_addr();

        let borrowed_socket_addr = socket_addr.clone();
        let (future_confirmation_sender, future_confirmation_receiver) = oneshot::channel::<bool>();
        let fut1 = tokio::spawn(async move {
            future_confirmation_sender.send(true).unwrap();
            tcp_listener
                .receive_from_client(&borrowed_socket_addr)
                .await
        });

        future_confirmation_receiver.await.unwrap();

        let data = vec![
            (
                b"some other payload".as_slice().to_vec(),
                random_socket_addr(),
            ),
            (b"Hello world".as_slice().to_vec(), socket_addr),
        ];

        for ele in &data {
            event_bus
                .send(RLPGEvent::NewPacketReceived(ele.clone()))
                .unwrap();
        }

        let fut1 = tokio::join!(fut1);
        let future_result = fut1.0.unwrap().unwrap();

        assert!(future_result == data[1].0.clone());
    }

    #[tokio::test]
    pub async fn test_if_tcp_listener_receives_from_any_client_when_simulated() {
        let cancellation_token = CancellationToken::new();
        let mut tcp_listener = TcpListenerGateway::run(
            &format!("0.0.0.0:{}", get_next_socket_port().await),
            cancellation_token.clone(),
        )
        .await
        .unwrap();
        let mut event_bus = tcp_listener.event_bus.clone();

        // wait until future is ready
        let (future_confirmation_sender, future_confirmation_receiver) = oneshot::channel::<bool>();
        let fut1 = tokio::spawn(async move {
            future_confirmation_sender.send(true).unwrap();
            tcp_listener.receive_from_all().await
        });

        future_confirmation_receiver.await.unwrap();

        let data = vec![
            (
                b"some other payload".as_slice().to_vec(),
                random_socket_addr(),
            ),
            (b"Hello world".as_slice().to_vec(), random_socket_addr()),
        ];

        for ele in &data {
            event_bus
                .send(RLPGEvent::NewPacketReceived(ele.clone()))
                .unwrap();
        }

        let fut = tokio::join!(fut1).0.unwrap();
        assert!(fut.ok().unwrap().0 == data[0].0);
    }

    pub fn test_if_tcp_listener_gateway_send_to_particular_client_function_works_correctly() {}
}
