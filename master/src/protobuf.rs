pub mod master {
    use proto::ModelType;

    pub fn hello_command_ok() -> proto::MasterPacket {
        proto::MasterPacket {
            msg: Some(proto::master_packet::Msg::HelloCommandOk(0)),
        }
    }

    pub fn load_command(model: ModelType) -> proto::MasterPacket {
        proto::MasterPacket {
            msg: Some(proto::master_packet::Msg::LoadCommand(model.into())),
        }
    }

    pub fn load_command_response_ack(model: ModelType) -> proto::MasterPacket {
        proto::MasterPacket {
            msg: Some(proto::master_packet::Msg::LoadCommandResponseAck(
                model.into(),
            )),
        }
    }

    pub fn generate_command(prompt: String) -> proto::MasterPacket {
        proto::MasterPacket {
            msg: Some(proto::master_packet::Msg::GenerateCommand(prompt)),
        }
    }
}

pub mod worker {
    pub struct HelloCommand;

    impl HelloCommand {
        pub fn new(name: String) -> proto::WorkerPacket {
            proto::WorkerPacket {
                msg: Some(proto::worker_packet::Msg::HelloCommand(
                    proto::HelloCommand { name },
                )),
            }
        }
    }
}
