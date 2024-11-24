use crate::tcp::connection::{ConnectionReader, ConnectionWriter};
use bytes::BytesMut;
use tokio::io::{BufReader, BufWriter};
use tokio::net::{TcpStream, ToSocketAddrs};

pub mod connection;

pub struct Client {
    pub reader: ConnectionReader,
    pub writer: ConnectionWriter,
}

impl Client {
    #[tracing::instrument(skip_all)]
    pub async fn connect<T: ToSocketAddrs>(addr: T) -> anyhow::Result<Client> {
        let socket = TcpStream::connect(addr).await?;
        tracing::info!("Connected to master");

        let (r, w) = socket.into_split();
        Ok(Client {
            reader: ConnectionReader {
                br: BufReader::new(r),
                buffer: BytesMut::with_capacity(4096),
            },
            writer: ConnectionWriter {
                bw: BufWriter::new(w),
            },
        })
    }
}
