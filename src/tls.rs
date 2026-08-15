use std::{error::Error, fs, path::Path};

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};

pub mod ca {
    use super::*;

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

    pub fn write(dir: &Path, ca: &CertifiedIssuer<'_, KeyPair>) -> Result<(), Box<dyn Error>> {
        fs::write(dir.join("ca.pem"), ca.pem())?;
        fs::write(dir.join("ca-key.pem"), ca.key().serialize_pem())?;
        Ok(())
    }
}

pub mod leaf {
    use super::*;

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
}
