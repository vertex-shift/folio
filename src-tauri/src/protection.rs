use std::{collections::BTreeMap, sync::Arc};

use lopdf::{
    Document, LoadOptions, Object,
    encryption::{
        DecryptionError, EncryptionState, EncryptionVersion, Permissions,
        crypt_filters::{Aes256CryptFilter, CryptFilter},
    },
};
use serde::{Deserialize, Serialize};

use crate::document::{Error, MAX_DOCUMENT_BYTES, Result, invalid};

pub(crate) struct Loaded {
    pub document: Document,
    pub copy_allowed: bool,
    pub editable: bool,
    pub permissions: AccessPermissions,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessPermissions {
    pub owner_authenticated: bool,
    pub print_allowed: bool,
    pub high_quality_print_allowed: bool,
    pub copy_allowed: bool,
    pub modify_allowed: bool,
    pub annotate_allowed: bool,
    pub fill_forms_allowed: bool,
    pub assemble_allowed: bool,
}

impl AccessPermissions {
    fn unrestricted() -> Self {
        Self {
            owner_authenticated: true,
            print_allowed: true,
            high_quality_print_allowed: true,
            copy_allowed: true,
            modify_allowed: true,
            annotate_allowed: true,
            fill_forms_allowed: true,
            assemble_allowed: true,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrintPermission {
    None,
    Low,
    High,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CopyPermissions {
    pub print: PrintPermission,
    pub copy: bool,
    pub edit: bool,
    pub annotate: bool,
    pub fill_forms: bool,
    pub assemble: bool,
}

// Credentials deliberately implement neither Debug nor Serialize.
#[derive(Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CopyProtection {
    Protect {
        opening_password: String,
        owner_password: String,
        permissions: CopyPermissions,
    },
    Unprotect,
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
            permissions: AccessPermissions::unrestricted(),
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
    let header = authentication_header(&document)?;
    header
        .authenticate_password(password)
        .map_err(password_error)?;
    let owner = authenticate_owner(&document, password)?;
    let permissions = state.permissions();
    let required = Permissions::MODIFIABLE | Permissions::COPYABLE | Permissions::ANNOTABLE;
    let editable = owner
        || permissions.contains(required)
            && (state.revision() == 2 || permissions.contains(Permissions::ASSEMBLABLE));
    let allowed = |permission| owner || permissions.contains(permission);
    let access = AccessPermissions {
        owner_authenticated: owner,
        print_allowed: allowed(Permissions::PRINTABLE),
        high_quality_print_allowed: allowed(Permissions::PRINTABLE)
            && (state.revision() == 2 || allowed(Permissions::PRINTABLE_IN_HIGH_QUALITY)),
        copy_allowed: allowed(Permissions::COPYABLE),
        modify_allowed: allowed(Permissions::MODIFIABLE),
        annotate_allowed: allowed(Permissions::ANNOTABLE),
        fill_forms_allowed: allowed(Permissions::ANNOTABLE)
            || state.revision() > 2 && allowed(Permissions::FILLABLE),
        assemble_allowed: allowed(Permissions::ASSEMBLABLE)
            || state.revision() == 2 && allowed(Permissions::MODIFIABLE),
    };
    Ok(Loaded {
        document,
        copy_allowed: access.copy_allowed,
        editable,
        permissions: access,
    })
}

fn authentication_header(document: &Document) -> Result<Document> {
    let state = document
        .encryption_state
        .as_ref()
        .ok_or_else(|| invalid("This document does not use password protection."))?;
    let mut header = Document::new();
    if let Ok(id) = document.trailer.get(b"ID") {
        header
            .trailer
            .set("ID", document.dereference(id)?.1.clone());
    }
    let encrypt = header.add_object(state.encode().map_err(lopdf::Error::from)?);
    header.trailer.set("Encrypt", encrypt);
    Ok(header)
}

pub fn authenticate_owner(document: &Document, password: &str) -> Result<bool> {
    if password.len() > 1024 {
        return Err(invalid("The owner password exceeds 1,024 UTF-8 bytes."));
    }
    match authentication_header(document)?.authenticate_owner_password(password) {
        Ok(()) => Ok(true),
        Err(lopdf::Error::Decryption(DecryptionError::IncorrectPassword))
        | Err(lopdf::Error::InvalidPassword) => Ok(false),
        Err(error) => Err(password_error(error)),
    }
}

pub(crate) fn author_copy(
    source: &Document,
    owner_authenticated: bool,
    authorization_password: Option<&str>,
    protection: &CopyProtection,
) -> Result<Vec<u8>> {
    if source.encryption_state.is_some() && !owner_authenticated {
        let password = authorization_password.ok_or_else(|| {
            invalid("Enter the current owner password to remove or replace this PDF's protection.")
        })?;
        if !authenticate_owner(source, password)? {
            return Err(invalid(
                "That is not the current owner password. The original and its protection are unchanged.",
            ));
        }
    }
    let mut output = source.clone();
    output.encryption_state = None;
    output.trailer.remove(b"Encrypt");
    output.prune_objects();
    if let CopyProtection::Protect {
        opening_password,
        owner_password,
        permissions,
    } = protection
    {
        if owner_password.is_empty() || owner_password == opening_password {
            return Err(invalid(
                "Set a nonempty owner password different from the opening password.",
            ));
        }
        if owner_password.len() > 127 || opening_password.len() > 127 {
            return Err(invalid(
                "New AES-256 passwords must be at most 127 UTF-8 bytes; Folio will not truncate them.",
            ));
        }
        let mut flags = Permissions::COPYABLE_FOR_ACCESSIBILITY;
        match permissions.print {
            PrintPermission::None => {}
            PrintPermission::Low => flags |= Permissions::PRINTABLE,
            PrintPermission::High => {
                flags |= Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY
            }
        }
        for (enabled, permission) in [
            (permissions.copy, Permissions::COPYABLE),
            (permissions.edit, Permissions::MODIFIABLE),
            (permissions.annotate, Permissions::ANNOTABLE),
            (permissions.fill_forms, Permissions::FILLABLE),
            (permissions.assemble, Permissions::ASSEMBLABLE),
        ] {
            if enabled {
                flags |= permission;
            }
        }
        let mut encryption_key = [0; 32];
        let mut file_id = [0; 16];
        getrandom::fill(&mut encryption_key)
            .and_then(|()| getrandom::fill(&mut file_id))
            .map_err(|_| {
                invalid(
                    "Operating-system randomness is unavailable; no protected copy was created.",
                )
            })?;
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
        let state = EncryptionState::try_from(EncryptionVersion::V5 {
            owner_password,
            user_password: opening_password,
            permissions: flags,
            encrypt_metadata: true,
            file_encryption_key: &encryption_key,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
        })?;
        output.trailer.set(
            "ID",
            vec![
                Object::string_literal(file_id),
                Object::string_literal(file_id),
            ],
        );
        output.version = "2.0".into();
        output.encryption_state = Some(state);
        if authenticate_owner(&output, opening_password)? {
            return Err(invalid(
                "The opening and owner passwords become identical after PDF password normalization. Choose different passwords.",
            ));
        }
        authentication_header(&output)?
            .authenticate_user_password(opening_password)
            .map_err(password_error)?;
    }
    serialize(&output)
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
