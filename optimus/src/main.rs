use std::io::{BufReader, Cursor};

use prost::Message;

mod onnx;

fn compile_protos() {
    prost_build::compile_protos(&["src/message.proto"], &["src/"]).unwrap();
}

fn main() {
    // compile_protos();

    let buf: Vec<u8> = std::fs::read("./src/model_fp16.onnx").unwrap();

    let model = onnx::ModelProto::decode(&buf[..]).unwrap();
    let graph = model.graph.unwrap();
    for node in graph.node {
        println!(
            "Name: {}; Inputs: {}; Output(s): {}; Operator type: {}",
            node.name.unwrap(),
            node.input.join(", "),
            node.output.join(", "),
            node.op_type.unwrap()
        );
    }
    // dbg!(model.training_info);
    //
    // około 100 roznych operacji
}
