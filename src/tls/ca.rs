//! Certificate authority creation and PEM output helpers.

use std::{error::Error, fs, path::Path};

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType, IsCa, KeyPair,
    KeyUsagePurpose,
};

/// Creates a self-signed certificate authority with `common_name` as its subject.
///
/// # Errors
/// Returns an error when rcgen cannot generate the CA private key or
/// self-signed certificate.
pub fn make(common_name: &str) -> Result<CertifiedIssuer<'static, KeyPair>, rcgen::Error> {
    let mut params = CertificateParams::default();

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    params.distinguished_name = dn;

    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];

    let ca_key = KeyPair::generate()?;
    CertifiedIssuer::self_signed(params, ca_key)
}

/// Writes a certificate authority certificate and private key as PEM files.
///
/// The files are written as `ca.pem` and `ca-key.pem` inside `dir`.
///
/// # Errors
/// Returns an error when either output file cannot be written.
pub fn write(dir: &Path, ca: &CertifiedIssuer<'_, KeyPair>) -> Result<(), Box<dyn Error>> {
    fs::write(dir.join("ca.pem"), ca.pem())?;
    fs::write(dir.join("ca-key.pem"), ca.key().serialize_pem())?;
    Ok(())
}
