
use micropb_gen::{ Generator };

// Generate Rust module from .proto files
fn proto_generate() {
    let mut gen = Generator::new();
    gen.use_container_heapless()
        .add_protoc_arg("-Iproto")
        .compile_protos(
            &[
                "aux.proto",
            ],
            std::env::var("OUT_DIR").unwrap() + "/aux.rs",
        )
        .unwrap();
    println!("cargo:rerun-if-changed=proto");
}

fn main() {
    proto_generate();
}