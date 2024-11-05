use prost::Message;
use proto::{Command, Load, ModelType, Package};

fn main() {
    let p1 = Package::new(Command::LoadCommand(Load::new(ModelType::Llama3v2_1B)));
    let mut buf = Vec::new();
    p1.encode(&mut buf).unwrap();

    let p2 = Package::decode(&mut buf.as_slice()).unwrap();
    println!("{:?}", p2);
}
