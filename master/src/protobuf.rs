pub mod master {
    pub fn hello_command_ok() -> proto::MasterPacket {
        proto::MasterPacket {
            msg: Some(proto::master_packet::Msg::HelloCommandOk(0)),
        }
    }

    pub fn load_command() -> proto::MasterPacket {
        proto::MasterPacket {
            msg: Some(proto::master_packet::Msg::LoadCommand(
                proto::ModelType::Llama3v21b.into(),
            )),
        }
    }
}

pub mod worker {
    pub struct HelloCommand;

    impl HelloCommand {
        pub fn new(name: String) -> proto::WorkerPacket {
            proto::WorkerPacket {
                msg: Some(proto::worker_packet::Msg::HelloCommand(proto::HelloCommand {
                    name,
                }))
            }
        }
    }
}
