fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rerun-if-changed=src");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::path::PathBuf::from(format!("{}/gen/include", manifest_dir));

    if out_dir.exists() {
        std::fs::remove_dir_all(out_dir.clone())?;
    }
    std::fs::create_dir_all(out_dir.clone())?;

    let mut config = cbindgen::Config::default();
    config.pragma_once = true;
    config.braces = cbindgen::Braces::NextLine;
    config.language = cbindgen::Language::C;
    config.line_length = 120;
    config.documentation_style = cbindgen::DocumentationStyle::C;
    config.cpp_compat = true;

    let mut server_config = config.clone();
    server_config.namespaces = Some(vec![String::from("grpc_web"), String::from("server")]);
    cbindgen::Builder::new()
        .with_src(std::path::PathBuf::from(&manifest_dir).join("src/server/mod.rs"))
        .with_config(server_config)
        .generate()
        .expect("Unable to generate bindings for server FFI")
        .write_to_file(out_dir.join("server.h"));

    let mut tls_config = config.clone();
    tls_config.namespaces = Some(vec![String::from("grpc_web"), String::from("tls")]);
    cbindgen::Builder::new()
        .with_src(std::path::PathBuf::from(&manifest_dir).join("src/tls/mod.rs"))
        .with_config(tls_config)
        .generate()
        .expect("Unable to generate bindings for TLS FFI")
        .write_to_file(out_dir.join("tls.h"));

    Ok(())
}
