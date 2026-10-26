use std::collections::BTreeMap;

use lopdf::{
    Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};

use super::{
    Coverage, Integrity, RESERVED_CMS_BYTES, crypto,
    fields::{Field, Widget},
    pdf,
};
use crate::document::{MAX_DOCUMENT_BYTES, Result, invalid, numbers};

fn claimed_time() -> Result<(String, String)> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid("The computer clock is before the supported signing date range."))?;
    let time = der::DateTime::from_unix_duration(duration)
        .map_err(|_| invalid("The computer clock is outside the supported signing date range."))?;
    Ok((
        format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minutes(),
            time.seconds(),
        ),
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minutes(),
            time.seconds(),
        ),
    ))
}

fn appearance(
    document: &mut Document,
    widget: &Widget,
    font: ObjectId,
    encoded: &BTreeMap<char, u16>,
    lines: &[String],
) -> Result<ObjectId> {
    let width = (widget.rect[2] - widget.rect[0]) * widget.geometry.user_unit;
    let height = (widget.rect[3] - widget.rect[1]) * widget.geometry.user_unit;
    let (width, height) = if widget.geometry.rotation % 180 == 0 {
        (width, height)
    } else {
        (height, width)
    };
    let line_width = lines
        .iter()
        .map(|line| crate::writing::line_metrics(line, 1.0).map(|metrics| metrics.0))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .fold(0.0_f32, f32::max);
    let size = 10.0_f32
        .min((width - 8.0) / line_width)
        .min((height - 8.0) / 5.6);
    if !size.is_finite() || size < 2.0 {
        return Err(invalid(
            "The selected widget is too small for a readable signature appearance.",
        ));
    }
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "re",
            numbers(&[0.0, 0.0, width, height]).as_array()?.clone(),
        ),
        Operation::new("W", vec![]),
        Operation::new("n", vec![]),
        Operation::new("rg", vec![1.into(), 1.into(), 1.into()]),
        Operation::new(
            "re",
            numbers(&[0.0, 0.0, width, height]).as_array()?.clone(),
        ),
        Operation::new("f", vec![]),
        Operation::new("RG", vec![0.2.into(), 0.2.into(), 0.2.into()]),
        Operation::new("w", vec![0.5.into()]),
        Operation::new(
            "re",
            numbers(&[0.5, 0.5, width - 1.0, height - 1.0])
                .as_array()?
                .clone(),
        ),
        Operation::new("S", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("rg", vec![0.1.into(), 0.1.into(), 0.1.into()]),
        Operation::new(
            "Tf",
            vec![Object::Name(b"FolioSignatureFont".to_vec()), size.into()],
        ),
        Operation::new("TL", vec![(size * 1.35).into()]),
        Operation::new("Td", vec![4.0.into(), (height - 4.0 - size * 1.1).into()]),
    ];
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            operations.push(Operation::new("T*", vec![]));
        }
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(
                line.chars()
                    .flat_map(|character| encoded[&character].to_be_bytes())
                    .collect(),
                StringFormat::Hexadecimal,
            )],
        ));
    }
    operations.extend([Operation::new("ET", vec![]), Operation::new("Q", vec![])]);
    Ok(document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => numbers(&[0.0, 0.0, width, height]),
            "Matrix" => numbers(&widget.geometry.appearance_matrix(width, height)),
            "Resources" => dictionary! { "Font" => dictionary! { "FolioSignatureFont" => font } },
        },
        Content { operations }.encode()?,
    )))
}

fn form_update(
    source: &Document,
    output: &mut IncrementalDocument,
    new_field: Option<ObjectId>,
) -> Result<()> {
    let (id, mut form) = match source.catalog()?.get(b"AcroForm") {
        Ok(value) if !matches!(value, Object::Null) => {
            let (id, value) = source.dereference(value)?;
            (id, value.as_dict()?.clone())
        }
        _ => (None, Dictionary::new()),
    };
    if let Some(field) = new_field {
        let (fields_id, mut fields) = match form.get(b"Fields") {
            Ok(value) if !matches!(value, Object::Null) => {
                let (id, value) = source.dereference(value)?;
                (id, value.as_array()?.clone())
            }
            _ => (None, vec![]),
        };
        fields.push(field.into());
        if let Some(id) = fields_id {
            output.new_document.set_object(id, Object::Array(fields));
        } else {
            form.set("Fields", fields);
        }
    }
    let flags = match form.get(b"SigFlags") {
        Ok(value) => source.dereference(value)?.1.as_i64()?,
        Err(_) => 0,
    };
    form.set("SigFlags", flags | 3);
    if let Some(id) = id {
        output.new_document.set_object(id, form);
    } else {
        let form = output.new_document.add_object(form);
        let root = source.trailer.get(b"Root")?.as_reference()?;
        output.opt_clone_object_to_new_document(root)?;
        output
            .new_document
            .get_dictionary_mut(root)?
            .set("AcroForm", form);
    }
    Ok(())
}

pub(super) fn signed_copy(
    source: &Document,
    original: &[u8],
    pfx: &[u8],
    password: &str,
    selected: Option<&Field>,
) -> Result<Vec<u8>> {
    let identity = crypto::identity(pfx, password)?;
    let signer = crypto::identity_info(&identity)?;
    let (pdf_time, display_time) = claimed_time()?;
    let mut output = IncrementalDocument::create_from(original.to_vec(), source.clone());
    let signature = output.new_document.add_object(dictionary! {
        "Type" => "Sig", "Filter" => "Adobe.PPKLite", "SubFilter" => "adbe.pkcs7.detached",
        "Name" => lopdf::text_string(&signer.subject),
        "M" => Object::string_literal(pdf_time),
        "ByteRange" => vec![0.into(), 9_999_999_999_i64.into(), 9_999_999_999_i64.into(), 9_999_999_999_i64.into()],
        "Contents" => Object::String(vec![0; RESERVED_CMS_BYTES], StringFormat::Hexadecimal),
    });
    if let Some(field) = selected {
        output.opt_clone_object_to_new_document(field.id)?;
        output
            .new_document
            .get_dictionary_mut(field.id)?
            .set("V", signature);
        let lines = vec![
            "Digitally signed".to_owned(),
            format!("Certificate: {}", &signer.sha256_fingerprint[..23]),
            format!("Local time: {display_time}"),
            "Not a trusted timestamp".to_owned(),
        ];
        let font = if field.info.widgets.iter().any(|widget| widget.visible) {
            Some(crate::writing::embed_text_font(
                &mut output.new_document,
                &lines.join(" "),
            )?)
        } else {
            None
        };
        for (widget, info) in field.widgets.iter().zip(&field.info.widgets) {
            output.opt_clone_object_to_new_document(widget.id)?;
            let ap = if info.visible {
                let (font, encoded) = font
                    .as_ref()
                    .ok_or_else(|| invalid("The signature appearance font is missing."))?;
                Some(appearance(
                    &mut output.new_document,
                    widget,
                    *font,
                    encoded,
                    &lines,
                )?)
            } else {
                None
            };
            let widget = output.new_document.get_dictionary_mut(widget.id)?;
            widget.remove(b"AS");
            if let Some(ap) = ap {
                widget.set("AP", dictionary! { "N" => ap });
            } else {
                widget.remove(b"AP");
            }
        }
        form_update(source, &mut output, None)?;
    } else {
        let page = source
            .get_pages()
            .into_values()
            .next()
            .ok_or_else(|| invalid("The PDF has no page for an approval signature."))?;
        let name = (0..4096)
            .map(|suffix| format!("FolioApproval{}_{}", signature.0, suffix))
            .find(|name| {
                !source.objects.values().any(|object| {
                    object
                        .as_dict()
                        .ok()
                        .and_then(|dictionary| dictionary.get(b"T").ok())
                        .and_then(|value| lopdf::decode_text_string(value).ok())
                        .as_deref()
                        == Some(name)
                })
            })
            .ok_or_else(|| {
                invalid("A unique approval signature field name could not be allocated.")
            })?;
        let field = output.new_document.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
            "T" => Object::string_literal(name), "V" => signature, "P" => page,
            "Rect" => vec![0.into(), 0.into(), 0.into(), 0.into()], "F" => 132,
        });
        let mut annotations = match source.get_dictionary(page)?.get(b"Annots") {
            Ok(value) => source.dereference(value)?.1.as_array()?.clone(),
            Err(_) => vec![],
        };
        annotations.push(field.into());
        output.opt_clone_object_to_new_document(page)?;
        output
            .new_document
            .get_dictionary_mut(page)?
            .set("Annots", annotations);
        form_update(source, &mut output, Some(field))?;
    }
    let mut bytes = Vec::new();
    output.save_to(&mut bytes)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("The signed copy exceeds the 128 MiB PDF limit."));
    }
    if !bytes.starts_with(original) {
        return Err(invalid(
            "The incremental writer did not preserve every original PDF byte.",
        ));
    }
    let loaded = crate::protection::load(&bytes, None)?.document;
    pdf::validate_footer(&bytes, &loaded)?;
    if pdf::revision_prev(&bytes, &loaded)? != Some(source.xref_start) {
        return Err(invalid(
            "The incremental signature did not retain the previous cross-reference chain.",
        ));
    }
    let raw = pdf::signature_object(&bytes, &loaded, signature)?;
    let contents = raw.get(b"Contents")?.span.clone();
    let byte_range = raw.get(b"ByteRange")?.span.clone();
    let replacement = format!(
        "[0 {} {} {}]",
        contents.start,
        contents.end,
        bytes.len() - contents.end
    );
    if raw.offset < original.len()
        || byte_range.start < original.len()
        || contents.start < original.len()
        || replacement.len() > byte_range.len()
        || contents.len() != RESERVED_CMS_BYTES * 2 + 2
    {
        return Err(invalid(
            "The incremental writer produced incompatible signature placeholder bounds.",
        ));
    }
    bytes[byte_range.clone()].fill(b' ');
    bytes[byte_range.start..byte_range.start + replacement.len()]
        .copy_from_slice(replacement.as_bytes());
    let mut data = Vec::with_capacity(bytes.len() - contents.len());
    data.extend_from_slice(&bytes[..contents.start]);
    data.extend_from_slice(&bytes[contents.end..]);
    let cms = crypto::sign(&identity, &data)?;
    if cms.len() > RESERVED_CMS_BYTES {
        return Err(invalid(
            "The certificate chain/signature exceeds the 32 KiB signing reservation. No file was exported.",
        ));
    }
    const HEX: &[u8] = b"0123456789ABCDEF";
    for (index, byte) in cms.iter().enumerate() {
        bytes[contents.start + 1 + index * 2] = HEX[(byte >> 4) as usize];
        bytes[contents.start + 2 + index * 2] = HEX[(byte & 15) as usize];
    }
    if !bytes.starts_with(original) {
        return Err(invalid(
            "Signing attempted to change an earlier PDF revision. No file was exported.",
        ));
    }
    let final_document = crate::protection::load(&bytes, None)?.document;
    let previous = super::inspect(source, original, 0, 0)?;
    let report = super::inspect(&final_document, &bytes, 0, 0)?;
    if report.signatures.len() != previous.signatures.len() + 1
        || report
            .signatures
            .iter()
            .any(|info| info.integrity != Integrity::Intact)
        || !report.signatures.iter().any(|info| {
            info.id == super::fields::key(signature) && info.coverage == Coverage::WholeDocument
        })
        || previous.signatures.iter().any(|old| {
            !report.signatures.iter().any(|next| {
                old.id == next.id
                    && old.signed_bytes == next.signed_bytes
                    && next.coverage == Coverage::LaterChanges
            })
        })
    {
        return Err(invalid(
            "The signed copy did not preserve and verify every previous signature plus the new signature after reopening. No file was exported.",
        ));
    }
    let fields = super::fields::read(&final_document)?;
    if let Some(field) = selected
        && !fields.fields.iter().any(|next| {
            next.id == field.id
                && next.signature == Some(signature)
                && next.info.widgets.iter().map(|widget| &widget.id).eq(field
                    .info
                    .widgets
                    .iter()
                    .map(|widget| &widget.id))
        })
    {
        return Err(invalid(
            "The signed copy did not retain the selected signature field/widget identity.",
        ));
    }
    Ok(bytes)
}
