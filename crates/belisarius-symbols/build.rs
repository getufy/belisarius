use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_dir = PathBuf::from("proto");
    let proto_path = proto_dir.join("scip.proto");
    println!("cargo:rerun-if-changed={}", proto_path.display());

    // Pure-Rust protobuf compiler — no system protoc required.
    let file_descriptors = protox::compile([proto_path], [proto_dir])?;
    // Strip comments from generated code — scip.proto includes pseudo-code
    // examples in its docs which rustc's doctest runner tries to compile.
    let mut cfg = prost_build::Config::new();
    cfg.disable_comments(["."]);
    cfg.compile_fds(file_descriptors)?;
    Ok(())
}
