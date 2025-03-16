fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=compiled");
    println!("cargo:rerun-if-changed=src/main.rs");

    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .file_descriptor_set_path("./compiled/service_descriptor.bin")
        .out_dir("./compiled")
        .compile_protos(&["protobufs/geirmund.proto"], &["protobufs"])
        .unwrap()
}
