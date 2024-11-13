use prost::Message;
use proto::master::{Generate, Load, MasterMessage, ModelType, Packet};

fn main() {
    let p1 = Packet::new(MasterMessage::GenerateCommand(Generate::new(
        1,
        ModelType::Llama3v2_1B,
        "write me a poem".to_string(),
    )));
    let mut buf = Vec::new();
    p1.encode(&mut buf).unwrap();
    buf.iter().for_each(|byte| println!("{}", format!("{:02x}", byte)));

    let p2 = Packet::decode(&mut buf.as_slice()).unwrap();
    println!("{:?}", p2);
}
