pub mod master {
    pub struct HelloCommandResponse {
    }

    impl HelloCommandResponse {
        pub fn ok(current_name: String) -> proto::MasterPacket {
            proto::MasterPacket { 
                msg: Some(
                    proto::master_packet::Msg::HelloCommandResponse(
                        proto::HelloCommandResponse {
                            result: Some(
                                proto::hello_command_response::Result::Ok(current_name)
                            )
                        }
                    )
                )
            }
        }

        pub fn you_already_have_a_name_error(current_name: String) -> proto::MasterPacket {
            proto::MasterPacket { 
                msg: Some(
                    proto::master_packet::Msg::HelloCommandResponse(
                        proto::HelloCommandResponse {
                            result: Some(
                                proto::hello_command_response::Result::Error(
                                    proto::HelloCommandResponseError {
                                        variant: Some(
                                            proto::hello_command_response_error::Variant::YouAlreadyHaveAName(
                                                proto::YouAlreadyHaveANameError {
                                                    name: current_name
                                                }
                                            )
                                        )
                                    }
                                )
                            )
                        }
                    )
                )
            }
        }

        pub fn worker_with_given_name_already_exists(name: String) -> proto::MasterPacket {
            proto::MasterPacket { 
                msg: Some(
                    proto::master_packet::Msg::HelloCommandResponse(
                        proto::HelloCommandResponse {
                            result: Some(
                                proto::hello_command_response::Result::Error(
                                    proto::HelloCommandResponseError {
                                        variant: Some(
                                            proto::hello_command_response_error::Variant::WorkerWithGivenNameAlreadyExists(
                                                name
                                            )
                                        )
                                    }
                                )
                            )
                        }
                    )
                )
            }
        }
    }
}
