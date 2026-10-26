#[path = "output_fixtures.rs"]
#[allow(dead_code)]
mod fixtures;

use std::{io::Write, path::PathBuf};

use folio_core::document::Session;
use lopdf::encryption::Permissions;
use openssl::{
    asn1::{Asn1Integer, Asn1Time},
    bn::BigNum,
    hash::MessageDigest,
    pkcs12::Pkcs12,
    pkey::PKey,
    rsa::Rsa,
    x509::{X509, X509NameBuilder, extension::KeyUsage},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Pass a synthetic-fixture output directory.")?,
    );
    std::fs::create_dir_all(&directory)?;
    let source = fixtures::synthetic_pdf(1, false);
    let key = PKey::from_rsa(Rsa::generate(2048)?)?;
    let mut name = X509NameBuilder::new()?;
    name.append_entry_by_text("CN", "Folio synthetic archival acceptance signer")?;
    let name = name.build();
    let mut certificate = X509::builder()?;
    certificate.set_version(2)?;
    let serial_number = BigNum::from_u32(42)?;
    let serial_number = Asn1Integer::from_bn(&serial_number)?;
    certificate.set_serial_number(&serial_number)?;
    certificate.set_subject_name(&name)?;
    certificate.set_issuer_name(&name)?;
    certificate.set_pubkey(&key)?;
    let not_before = Asn1Time::days_from_now(0)?;
    let not_after = Asn1Time::days_from_now(1)?;
    certificate.set_not_before(&not_before)?;
    certificate.set_not_after(&not_after)?;
    certificate.append_extension(KeyUsage::new().digital_signature().build()?)?;
    certificate.sign(&key, MessageDigest::sha256())?;
    let certificate = certificate.build();
    let password = "synthetic ephemeral test identity";
    let identity = zeroize::Zeroizing::new(
        Pkcs12::builder()
            .name("Synthetic only")
            .pkey(&key)
            .cert(&certificate)
            .build2(password)?
            .to_der()?,
    );
    let signed = Session::from_bytes(source.clone(), "Synthetic.pdf".into(), None, 1)?
        .signed_copy(&identity, password)?;
    for (name, bytes) in [
        (
            "owner-required.pdf",
            fixtures::encrypted(&source, 4, Permissions::all()),
        ),
        (
            "copy-restricted.pdf",
            fixtures::encrypted(&source, 4, Permissions::PRINTABLE),
        ),
        ("cryptographically-signed.pdf", signed),
    ] {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))?;
        file.write_all(&bytes)?;
    }
    println!(
        "Created original synthetic protected/signed fixtures. Opening password: user; owner: owner. No private key or PKCS#12 file was saved."
    );
    Ok(())
}
