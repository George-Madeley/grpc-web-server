use clap::Parser;
use grpc_web_server::tls;
use rcgen::ExtendedKeyUsagePurpose;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The path to store the key-cert pairs
    out_dir: std::path::PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    std::fs::create_dir_all(&args.out_dir)?;

    let ca = tls::ca::make("grpc-web-dev-ca")?;
    tls::ca::write(&args.out_dir, &ca)?;

    let server = tls::leaf::issue(
        &ca,
        "grpc-server",
        vec!["grpc.local".to_string()],
        ExtendedKeyUsagePurpose::ServerAuth,
    )?;
    tls::leaf::write(&args.out_dir, "server", &server)?;

    let proxy = tls::leaf::issue(
        &ca,
        "grpc-proxy",
        vec!["proxy.local".to_string()],
        ExtendedKeyUsagePurpose::ClientAuth,
    )?;
    tls::leaf::write(&args.out_dir, "proxy", &proxy)?;

    Ok(())
}
