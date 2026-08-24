//! Leaf-certificate issuance and PEM output helpers.

use std::{error::Error, fs, path::Path};

use rcgen::{
    CertificateParams, CertifiedIssuer, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};

/// Issues a leaf certificate from `ca` for the supplied names and usage.
///
/// `common_name` becomes the certificate subject, and `subject_alt_names`
/// becomes its Subject Alternative Name extension.
///
/// # Errors
/// Returns an error when rcgen cannot generate a private key, validate the
/// subject alternative names, or sign the certificate.
pub fn issue(
    ca: &CertifiedIssuer<'_, KeyPair>,
    common_name: &str,
    subject_alt_names: Vec<String>,
    eku: ExtendedKeyUsagePurpose,
) -> Result<(rcgen::Certificate, KeyPair), rcgen::Error> {
    let leaf_key = KeyPair::generate()?;
    let mut params = CertificateParams::new(subject_alt_names)?;

    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    params.distinguished_name = dn;

    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![eku];

    let cert = params.signed_by(&leaf_key, ca)?;
    Ok((cert, leaf_key))
}

/// Writes a leaf certificate and private key as PEM files.
///
/// The files are written as `{prefix}.pem` and `{prefix}-key.pem` inside
/// `dir`.
///
/// # Errors
/// Returns an error when either output file cannot be written.
pub fn write(
    dir: &Path,
    prefix: &str,
    leaf: &(rcgen::Certificate, KeyPair),
) -> Result<(), Box<dyn Error>> {
    fs::write(dir.join(format!("{prefix}.pem")), leaf.0.pem())?;
    fs::write(
        dir.join(format!("{prefix}-key.pem")),
        leaf.1.serialize_pem(),
    )?;
    Ok(())
}
