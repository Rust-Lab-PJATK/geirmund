use std::{future::Future, sync::Arc};

use rlpg::tcp::{RLPGEvent, RLPGEventBus, RLPGTcpListener, RunServerError};
use thiserror::Error;
use tokio::sync::broadcast::error::SendError;
use tokio_util::sync::CancellationToken;

use crate::event_bus::{Event, EventBus, ServerEvent};
use prost::Message;

#[derive(Error, Debug, Clone)]
pub enum ReactToRLPGEventError {
    #[error("protobuf decode error: {0}")]
    DecodeError(#[from] prost::DecodeError),

    #[error("send error occured: {0}")]
    SendError(Arc<SendError<Event>>),
}

impl From<SendError<Event>> for ReactToRLPGEventError {
    fn from(value: SendError<Event>) -> Self {
        return ReactToRLPGEventError::SendError(Arc::new(value));
    }
}

fn react_to_rlpg_event(
    event_bus: &mut EventBus,
    rlpg_event_bus: &mut RLPGEventBus,
    event: RLPGEvent,
) -> Option<proto::worker::Packet> {
    match event {
        RLPGEvent::ClientConnected(addr) => {
            event_bus
                .send(Event::Server(ServerEvent::ClientConnected(addr)))
                .unwrap();

            return None;
        }
        RLPGEvent::NewPacket((packet, addr)) => {
            match proto::worker::Packet::decode(&mut packet.as_slice()) {
                Ok(v) => return Some(v),
                Err(e) => {
                    if let Err(send_err) = rlpg_event_bus.send(RLPGEvent::DisconnectTheClient(addr))
                    {
                        panic!("Error occured while trying to send DisconnectTheClient event: {send_err:?}, {e:?}")
                    }
                    panic!("Error occured while parsing protobuf packet from worker: {e:?}");
                }
            }
        }
        RLPGEvent::ClientDisconnected(_) => {
            if let Err(e) = event_bus.send(Event::Server(ServerEvent::ClientDisconnected)) {
                panic!("Error occured while trying to send event: {e:?}");
            }

            return None;
        }
        _ => return None,
    };
}

impl From<proto::worker::Packet> for Event {
    fn from(value: proto::worker::Packet) -> Self {
        match value.msg {
            Some(v) => match v {
                proto::worker::WorkerMessage::GenerateResponse(response) => {
                    return Event::Server(ServerEvent::WorkerGeneratedResponse(response.content));
                }
            },
            None => unimplemented!(),
        }
    }
}

pub fn run(
    addr: String,
    cancellation_token: CancellationToken,
    mut event_bus: EventBus,
) -> impl Future<Output = Result<(), RunServerError>> {
    async move {
        let mut rlpg_event_bus = RLPGEventBus::new();
        let rlpg_server = RLPGTcpListener::new(rlpg_event_bus.clone());

        let addr = addr.clone();
        let server_fut_cancel_token = cancellation_token.clone();

        // server_fut
        tokio::spawn(async move {
            // this weird thing is because of borrow checker complaints
            if let Err(e) = rlpg_server.run(&addr, server_fut_cancel_token).await {
                return Err(e);
            }

            return Ok(());
        });

        let rlpg_packet_parsing_fut_cancel_token = cancellation_token.clone();
        // rlpg_packet_parsing_fut
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = rlpg_packet_parsing_fut_cancel_token.cancelled() => return,
                    rlpg_event = rlpg_event_bus.receive() => {
                        let result = match react_to_rlpg_event(&mut event_bus, &mut rlpg_event_bus, rlpg_event) {
                            Some(packet) => packet,
                            None => continue,
                        };

                        if let Err(e) = event_bus.send(Event::from(result)) {
                            panic!("Error occured while trying to send event: {e:?}");
                        }
                    }
                }
            }
        });

        cancellation_token.cancelled().await;

        Ok(())
    }
}
