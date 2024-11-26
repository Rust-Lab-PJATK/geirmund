use anyhow::{bail, Error as E, Result};
use bytes::{Buf, BytesMut};
use prost::Message;
use proto::master::Packet as MasterPacket;
use proto::worker::Packet as WorkerPacket;
use std::io::Cursor;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

pub struct ConnectionReader {
    pub br: BufReader<OwnedReadHalf>,
    pub buffer: BytesMut,
}

pub struct ConnectionWriter {
    pub bw: BufWriter<OwnedWriteHalf>,
}

impl ConnectionReader {
    pub async fn read_packet(&mut self) -> Result<Option<MasterPacket>> {
        loop {
            if let Ok(packet) = self.parse_packet() {
                return Ok(Some(packet));
            }

            if self.br.read_buf(&mut self.buffer).await? == 0 {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                bail!("Connection reset by peer")
            }
        }
    }

    fn parse_packet(&mut self) -> Result<MasterPacket> {
        let mut buf = Cursor::new(&self.buffer[..]);
        let packet = MasterPacket::decode(&mut buf);

        if let Ok(ref p) = packet {
            if p.msg.is_none() {
                bail!("Packet malformed")
            }
            self.buffer.advance(p.encoded_len());
        }
        packet.map_err(E::msg)
    }
}

impl ConnectionWriter {
    pub async fn write_packet(&mut self, packet: &WorkerPacket) -> Result<()> {
        self.bw.write_all(packet.encode_to_vec().as_slice()).await?;
        self.bw.flush().await?;

        Ok(())
    }
}
