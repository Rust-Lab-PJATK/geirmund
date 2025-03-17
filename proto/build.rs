fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=compiled");
    println!("cargo:rerun-if-changed=src/main.rs");

    if !std::fs::exists("./compiled").unwrap() {
        std::fs::create_dir("./compiled").unwrap();
    }

    if !std::fs::exists("./compiled/service_descriptor.bin").unwrap() {
        std::fs::write("./compiled/service_descriptor.bin", "").unwrap();
    }

    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir("./compiled")
        .file_descriptor_set_path("./compiled/service_descriptor.bin")
        .compile_protos(&["protobufs/geirmund.proto"], &["protobufs"])
        .unwrap()
}
