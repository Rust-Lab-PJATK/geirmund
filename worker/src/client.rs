use anyhow::{bail, Error as E, Result};
use bytes::{Buf, BytesMut};
use prost::Message;
use proto::master::Packet;
use std::io::Cursor;
use tokio::io::{AsyncReadExt, BufWriter};
use tokio::net::{TcpStream, ToSocketAddrs};

pub struct Client {
    pub connection: Connection,
}

pub struct Connection {
    pub stream: BufWriter<TcpStream>,
    pub buffer: BytesMut,
}

impl Client {
    pub async fn connect<T: ToSocketAddrs>(addr: T) -> Result<Client> {
        let socket = TcpStream::connect(addr).await?;
        let connection = Connection::new(socket);
        Ok(Client { connection })
    }
}

impl Connection {
    pub fn new(socket: TcpStream) -> Self {
        Self {
            stream: BufWriter::new(socket),
            buffer: BytesMut::with_capacity(4 * 1024),
        }
    }

    pub async fn read_packet(&mut self) -> Result<Option<Packet>> {
        loop {
            if let Ok(packet) = self.parse_packet() {
                return Ok(Some(packet));
            }

            if self.stream.read_buf(&mut self.buffer).await? == 0 {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                bail!("Connection reset by peer")
            }
        }
    }

    fn parse_packet(&mut self) -> Result<Packet> {
        let mut buf = Cursor::new(&self.buffer[..]);
        let packet = Packet::decode(&mut buf);

        if let Ok(ref p) = packet {
            if p.msg.is_none() {
                bail!("Packet malformed")
            }
            self.buffer.advance(p.encoded_len());
        }
        packet.map_err(E::msg)
    }
}
