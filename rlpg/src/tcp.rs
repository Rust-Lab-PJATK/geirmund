use std::future::Future;
use std::net::SocketAddr;
use tokio_util::sync::CancellationToken;

pub mod error {
    pub mod tcp_listener {
        pub enum ReceiveFromClientError {}

        pub enum ReceiveFromAllError {}

        pub enum SendToAllError {}

        pub enum SendToClientError {}

        pub enum RunError {}
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
pub struct TcpListener {
    //TODO
}

impl TcpListener {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn receive_from_client(
        &self,
    ) -> Result<Vec<u8>, error::tcp_listener::ReceiveFromClientError> {
        //TODO
        Ok(Vec::new())
    }

    pub async fn receive_from_all(
        &self,
    ) -> Result<Vec<u8>, error::tcp_listener::ReceiveFromAllError> {
        //TODO
        Ok(Vec::new())
    }

    pub async fn send_to_client(
        &self,
        client_addr: SocketAddr,
        payload: Vec<u8>,
    ) -> Result<(), error::tcp_listener::SendToClientError> {
        //TODO
        Ok(())
    }

    pub async fn send_to_all(
        &self,
        payload: Vec<u8>,
    ) -> Result<(), error::tcp_listener::SendToAllError> {
        //TODO
        Ok(())
    }

    pub fn run(
        &self,
        addr: &str,
        cancellation_token: CancellationToken,
    ) -> impl Future<Output = Result<(), error::tcp_listener::RunError>> {
        async move { Ok(()) }
    }

    pub async fn listen_for_new_connection(&self) -> SocketAddr {
        SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )
    }

    pub async fn listen_for_disconnect(&self) -> SocketAddr {
        SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )
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

//pub struct RLPGTcpListener {
//    event_bus: RLPGEventBus,
//}
//
//async fn disconnect_the_client_on_event(
//    event: &RLPGEvent,
//    socket: &mut TcpStream,
//    socket_event_bus: &mut RLPGEventBus,
//    socket_addr: &SocketAddr,
//) -> Result<bool, RunServerError> {
//    match event {
//        RLPGEvent::DisconnectTheClient(addr) => {
//            if *addr != *socket_addr {
//                return Ok(false);
//            }
//
//            let _ = socket.shutdown().await;
//            socket_event_bus.send(RLPGEvent::ClientDisconnected((
//                DisconnectReason::Unknown,
//                *socket_addr,
//            )))?;
//            return Ok(true);
//        }
//        _ => return Ok(false),
//    }
//}
//
//async fn send_new_packet_on_event(
//    event: &RLPGEvent,
//    socket: &mut TcpStream,
//    socket_addr: &SocketAddr,
//) -> Result<(), RunServerError> {
//    match event {
//        RLPGEvent::SendNewPacketToParticularClient((content, target_socket_addr)) => {
//            if *target_socket_addr != *socket_addr {
//                return Ok(());
//            }
//
//            let content_length = content.len();
//            let header = format!("RLPG/{VERSION}\n{content_length}\n\n");
//            socket.write_all(&header.as_bytes()).await?;
//            socket.write_all(&content).await?;
//        }
//        RLPGEvent::SendNewPacket(content) => {
//            let content_length = content.len();
//            let header = format!("RLPG/{VERSION}\n{content_length}\n\n");
//            socket.write_all(&header.as_bytes()).await?;
//            socket.write_all(&content).await?;
//        }
//        _ => {}
//    };
//
//    Ok(())
//}
//
//async fn run_on_socket(
//    mut socket: TcpStream,
//    cancellation_token: CancellationToken,
//    mut socket_event_bus: RLPGEventBus,
//) -> impl Future<Output = Result<(), RunServerError>> {
//    async move {
//        let socket_addr = socket.peer_addr()?;
//        let mut parser = RLPGParser::new();
//
//        loop {
//            tokio::select! {
//                _ = cancellation_token.cancelled() => return Ok(()),
//                    res = socket.read_u8() => {
//                    match res {
//                        Ok(character) => parser.add_char_to_buffer(character),
//                        Err(e) => {
//                            let _ = socket.shutdown().await;
//                            socket_event_bus.send(RLPGEvent::ClientDisconnected((
//                                DisconnectReason::Unknown,
//                                socket_addr,
//                            )))?;
//                            return Err(RunServerError::IOError(Arc::new(e)));
//                        }
//                    };
//                },
//                    ev = socket_event_bus.receive() => {
//                    if disconnect_the_client_on_event(&ev, &mut socket, &mut socket_event_bus, &socket_addr).await? {
//                        return Ok(());
//                    }
//
//                    send_new_packet_on_event(&ev, &mut socket, &socket_addr).await?;
//
//                    continue;
//                },
//            };
//
//            match parser.parse() {
//                Ok(Some(parsed_data)) => {
//                    socket_event_bus
//                        .sender
//                        .send(RLPGEvent::NewPacketReceived((parsed_data, socket_addr)))?;
//
//                    parser = RLPGParser::new();
//                }
//                Err(e) => {
//                    let _ = socket.shutdown().await;
//                    socket_event_bus.send(RLPGEvent::ClientDisconnected((
//                        DisconnectReason::InvalidByte,
//                        socket_addr,
//                    )))?;
//                    return Ok(());
//                }
//                _ => {}
//            };
//        }
//    }
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
//        let listener = TcpListener::bind(addr).await?;
//        loop {
//            let (socket, addr) = tokio::select! {
//                res = listener.accept() => res?,
//                    _ = cancellation_token.cancelled() => return Ok(())
//            };
//
//            let cancellation_token = cancellation_token.clone();
//
//            let mut socket_event_bus = self.event_bus.clone();
//
//            if let Err(e) = socket_event_bus.send(RLPGEvent::ClientConnected(addr)) {
//                panic!(
//                    "Error occured while trying to send RLPGEvent::ClientConnected event: {e:?}"
//                );
//            }
//
//            tokio::spawn(run_on_socket(socket, cancellation_token, socket_event_bus));
//        }
//    }
//}
//
//#[derive(Debug, Clone, Copy, PartialEq, Eq)]
//pub enum DisconnectReason {
//    InvalidByte,
//    Unknown,
//}
//
//#[derive(Debug, Clone, PartialEq, Eq)]
//enum RLPGEvent {
//    ClientConnected(SocketAddr),
//    NewPacketReceived((Vec<u8>, SocketAddr)),
//    SendNewPacket((Vec<u8>, Option<SocketAddr>)),
//    ClientDisconnected((DisconnectReason, SocketAddr)),
//    DisconnectTheClient(SocketAddr),
//}
//
//#[derive(Debug)]
//struct RLPGEventBus {
//    sender: Sender<RLPGEvent>,
//    consumer: Receiver<RLPGEvent>,
//}
//
//impl Clone for RLPGEventBus {
//    fn clone(&self) -> Self {
//        RLPGEventBus {
//            sender: self.sender.clone(),
//            consumer: self.sender.subscribe(),
//        }
//    }
//}
//
//impl RLPGEventBus {
//    pub fn new() -> Self {
//        let (sender, consumer) = tokio::sync::broadcast::channel(100);
//        RLPGEventBus { sender, consumer }
//    }
//
//    pub fn send(
//        &mut self,
//        value: RLPGEvent,
//    ) -> Result<usize, Arc<tokio::sync::broadcast::error::SendError<RLPGEvent>>> {
//        self.sender.send(value).map_err(|err| Arc::new(err))
//    }
//
//    pub async fn receive(&mut self) -> RLPGEvent {
//        loop {
//            match self.consumer.recv().await {
//                Ok(v) => return v,
//                _ => {}
//            }
//        }
//    }
//}
//
//#[derive(Error, Debug, Clone)]
//pub enum RunServerError {
//    #[error("IO error: {0}")]
//    IOError(Arc<tokio::io::Error>),
//
//    #[error("Failed to send event through event bus: {0}")]
//    SendError(Arc<SendError<RLPGEvent>>),
//}
//
//impl From<SendError<RLPGEvent>> for RunServerError {
//    fn from(value: SendError<RLPGEvent>) -> Self {
//        return RunServerError::SendError(Arc::new(value));
//    }
//}
//
//impl From<Arc<SendError<RLPGEvent>>> for RunServerError {
//    fn from(value: Arc<SendError<RLPGEvent>>) -> Self {
//        return RunServerError::SendError(value);
//    }
//}
//
//impl From<tokio::io::Error> for RunServerError {
//    fn from(value: tokio::io::Error) -> Self {
//        return RunServerError::IOError(Arc::new(value));
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
