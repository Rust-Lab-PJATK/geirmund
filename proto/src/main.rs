use prost::Message;
use proto::master::{Load, MasterMessage, ModelType, Packet};

fn main() {
    let p1 = Packet::new(MasterMessage::LoadCommand(Load::new(
        ModelType::Llama3v2_1B,
    )));
    let mut buf = Vec::new();
    p1.encode(&mut buf).unwrap();

    let p2 = Packet::decode(&mut buf.as_slice()).unwrap();
    println!("{:?}", p2);
}
