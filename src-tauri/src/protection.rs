use lopdf::{
    Document, LoadOptions,
    encryption::{DecryptionError, Permissions},
};

use crate::document::{Error, MAX_DOCUMENT_BYTES, Result, invalid};

pub(crate) struct Loaded {
    pub document: Document,
    pub copy_allowed: bool,
    pub editable: bool,
}

fn password_error(error: lopdf::Error) -> Error {
    match error {
        lopdf::Error::InvalidPassword
        | lopdf::Error::Decryption(DecryptionError::IncorrectPassword) => {
            Error::PasswordRequired { incorrect: true }
        }
        lopdf::Error::Decryption(_) | lopdf::Error::UnsupportedSecurityHandler(_) => invalid(
            "This PDF uses unsupported or malformed encryption. Use another PDF reader to save a compatible copy.",
        ),
        error => error.into(),
    }
}

pub(crate) fn load(bytes: &[u8], password: Option<&str>) -> Result<Loaded> {
    if password.is_some_and(|value| value.len() > 1024) {
        return Err(invalid("The PDF password exceeds the 1,024-byte limit."));
    }
    let options = LoadOptions {
        password: password.map(str::to_owned),
        max_decompressed_size: Some(MAX_DOCUMENT_BYTES),
        ..Default::default()
    };
    let document = Document::load_mem_with_options(bytes, options).map_err(password_error)?;
    if document.is_encrypted() {
        if document.get_encrypted()?.get(b"Filter")?.as_name()? != b"Standard" {
            return Err(invalid(
                "This PDF uses certificate-based or unsupported protection, not a standard PDF password.",
            ));
        }
        match document.authenticate_password(password.unwrap_or("")) {
            Err(lopdf::Error::Decryption(DecryptionError::IncorrectPassword)) => {
                return Err(Error::PasswordRequired {
                    incorrect: password.is_some(),
                });
            }
            Err(error) => return Err(password_error(error)),
            Ok(()) => {
                return Err(invalid("The PDF could not be decrypted completely."));
            }
        }
    }
    let Some(state) = &document.encryption_state else {
        return Ok(Loaded {
            document,
            copy_allowed: true,
            editable: true,
        });
    };
    let Some(password) = password else {
        return Err(Error::PasswordRequired { incorrect: false });
    };
    if state.version() >= 4 {
        for filter in [state.default_stream_filter(), state.default_string_filter()] {
            if !state.crypt_filters().contains_key(filter) {
                return Err(invalid("This PDF uses an unsupported encryption filter."));
            }
        }
    }
    // Authenticate the supplied password even when the reader automatically used
    // an empty opening password. Owner access must not be inferred from that fallback.
    let mut header = Document::new();
    if let Ok(id) = document.trailer.get(b"ID") {
        header
            .trailer
            .set("ID", document.dereference(id)?.1.clone());
    }
    let encrypt = header.add_object(state.encode().map_err(lopdf::Error::from)?);
    header.trailer.set("Encrypt", encrypt);
    header
        .authenticate_password(password)
        .map_err(password_error)?;
    let owner = match header.authenticate_owner_password(password) {
        Ok(()) => true,
        Err(lopdf::Error::Decryption(DecryptionError::IncorrectPassword)) => false,
        Err(error) => return Err(password_error(error)),
    };
    let permissions = state.permissions();
    let required = Permissions::MODIFIABLE | Permissions::COPYABLE | Permissions::ANNOTABLE;
    let editable = owner
        || permissions.contains(required)
            && (state.revision() == 2 || permissions.contains(Permissions::ASSEMBLABLE));
    let copy_allowed = owner || permissions.contains(Permissions::COPYABLE);
    Ok(Loaded {
        document,
        copy_allowed,
        editable,
    })
}

pub(crate) fn serialize(document: &Document) -> Result<Vec<u8>> {
    let mut output = document.clone();
    if let Some(state) = output.encryption_state.take() {
        output.encrypt(&state)?;
    }
    let mut bytes = Vec::new();
    output.save_to(&mut bytes)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid(
            "The output PDF exceeds the 128 MiB file-size limit.",
        ));
    }
    Ok(bytes)
}

pub(crate) fn protect_export(source: &Document, bytes: Vec<u8>) -> Result<Vec<u8>> {
    let Some(state) = &source.encryption_state else {
        return Ok(bytes);
    };
    let mut output = Document::load_mem(&bytes)?;
    if state.version() <= 4 {
        // Legacy password authentication derives its key from the first file ID.
        let id = source.trailer.get(b"ID")?;
        output.trailer.set("ID", source.dereference(id)?.1.clone());
    }
    output.encryption_state = Some(state.clone());
    serialize(&output)
}
