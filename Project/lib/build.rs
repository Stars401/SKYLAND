fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vendored protoc — eliminates external protoc dependency
    let protoc_path = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc_path);

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/skyland.proto"], &["proto"])?;
    Ok(())
}
