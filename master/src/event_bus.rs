use std::net::SocketAddr;

use proto::master::ModelType;
use tokio::sync::broadcast::{
    self,
    error::{RecvError, SendError},
};

#[derive(Debug, Clone)]
pub enum TuiEvent {
    SelectedModel(ModelType),
}

#[derive(Debug, Clone)]
pub enum Event {
    Tui(TuiEvent),
    Server(ServerEvent),
}

#[derive(Debug, Clone)]
pub enum ServerEvent {
    WorkerLoadedModel(ModelType),
    WorkerGeneratedResponse(String),
    ClientConnected(SocketAddr),
    ClientDisconnected,
}

#[derive(Debug)]
pub struct EventBus {
    receiver: tokio::sync::broadcast::Receiver<Event>,
    sender: tokio::sync::broadcast::Sender<Event>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, receiver) = broadcast::channel(16);

        Self { receiver, sender }
    }

    pub fn clone_mut(&mut self) -> Self {
        Self {
            receiver: self.sender.subscribe(),
            sender: self.sender.clone(),
        }
    }

    pub async fn receive(&mut self) -> Result<Event, RecvError> {
        return self.receiver.recv().await;
    }

    pub fn send(&mut self, payload: Event) -> Result<usize, SendError<Event>> {
        return self.sender.send(payload);
    }
}
