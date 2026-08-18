fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::path::PathBuf::from(format!("{}/gen/include", manifest_dir));

    if out_dir.exists() {
        std::fs::remove_dir_all(out_dir.clone())?;
    }
    std::fs::create_dir_all(out_dir.clone())?;

    let mut config = cbindgen::Config::default();
    config.pragma_once = true;
    config.namespace = Some(String::from("grpc_web_server"));
    config.braces = cbindgen::Braces::NextLine;
    config.language = cbindgen::Language::C;
    config.line_length = 120;
    config.documentation_style = cbindgen::DocumentationStyle::C;
    config.cpp_compat = true;

    cbindgen::Builder::new()
        .with_crate(manifest_dir)
        .with_config(config)
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(out_dir.join("grpc_web_server.h"));

    Ok(())
}
