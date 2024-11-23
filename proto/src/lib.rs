pub mod master;
pub mod worker;

#[cfg(test)]
mod tests {
    use prost::Message;

    use crate::master::{Generate, MasterMessage, ModelType, Packet};

    #[test]
    pub fn it_encodes_and_decodes_to_the_same_data() {
        let packet = Packet::new(MasterMessage::GenerateCommand(Generate::new(
            1,
            ModelType::Llama3v2_1B,
            "write me a poem".to_string(),
        )));

        let mut buf = Vec::new();
        packet.encode(&mut buf).unwrap();

        let decoded_packet = Packet::decode(&mut buf.as_slice()).unwrap();

        assert_eq!(decoded_packet, packet)
    }
}
