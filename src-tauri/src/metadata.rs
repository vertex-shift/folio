use lopdf::{Dictionary, Document, Stream, text_string};
use quick_xml::{Writer, events::Event, name::ResolveResult, reader::NsReader};
use serde::{Deserialize, Serialize};

use crate::{
    document::{Result, invalid},
    pdf_objects::{resolve, stream_bytes, text, validate_text},
};

const MAX_XMP_BYTES: usize = 1024 * 1024;
const MAX_FIELD_BYTES: usize = 4096;
const RDF: &[u8] = b"http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const DC: &[u8] = b"http://purl.org/dc/elements/1.1/";
const PDF: &[u8] = b"http://ns.adobe.com/pdf/1.3/";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Fields {
    pub title: String,
    pub author: String,
    pub subject: String,
    pub keywords: String,
}

impl Fields {
    fn values(&self) -> [&str; 4] {
        [&self.title, &self.author, &self.subject, &self.keywords]
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataInfo {
    pub fields: Fields,
    pub xmp_present: bool,
    pub editable: bool,
    pub reason: Option<String>,
}

fn info_dictionary(document: &Document) -> Result<Dictionary> {
    document
        .trailer
        .get(b"Info")
        .ok()
        .map(|value| Ok(resolve(document, value)?.as_dict()?.clone()))
        .unwrap_or_else(|| Ok(Dictionary::new()))
}

fn xmp_stream(document: &Document) -> Result<Option<&Stream>> {
    document
        .catalog()?
        .get(b"Metadata")
        .ok()
        .map(|value| Ok(resolve(document, value)?.as_stream()?))
        .transpose()
}

pub(crate) fn inspect(document: &Document) -> Result<MetadataInfo> {
    let dictionary = info_dictionary(document)?;
    let mut values = [String::new(), String::new(), String::new(), String::new()];
    let keys: [&[u8]; 4] = [b"Title", b"Author", b"Subject", b"Keywords"];
    for (index, key) in keys.iter().enumerate() {
        if let Ok(value) = dictionary.get(key) {
            values[index] = text(document, value, MAX_FIELD_BYTES)?;
        }
    }
    let present = document.catalog()?.has(b"Metadata");
    let mut reason = None;
    if present {
        match xmp_stream(document).and_then(|stream| {
            let bytes = stream_bytes(
                stream.ok_or_else(|| invalid("The XMP metadata is missing."))?,
                MAX_XMP_BYTES,
            )?;
            transform_xmp(&bytes, None).map(|(_, fields)| fields)
        }) {
            Ok(fields) => {
                for (index, value) in fields.into_iter().enumerate() {
                    if !dictionary.has(keys[index]) {
                        values[index] = value;
                    }
                }
            }
            Err(error) => {
                reason = Some(format!(
                    "Descriptive metadata editing is unavailable because the existing XMP cannot be updated consistently: {error}"
                ))
            }
        }
    }
    let [title, author, subject, keywords] = values;
    Ok(MetadataInfo {
        fields: Fields {
            title,
            author,
            subject,
            keywords,
        },
        xmp_present: present,
        editable: reason.is_none(),
        reason,
    })
}

pub(crate) fn update(document: &mut Document, fields: Fields) -> Result<()> {
    for value in fields.values() {
        validate_text(value, MAX_FIELD_BYTES, true)?;
    }
    let xmp = xmp_stream(document)?
        .map(|stream| {
            let bytes = stream_bytes(stream, MAX_XMP_BYTES)?;
            let (bytes, _) = transform_xmp(&bytes, Some(&fields))?;
            let mut dictionary = stream.dict.clone();
            dictionary.remove(b"Filter");
            dictionary.remove(b"DecodeParms");
            Ok::<_, crate::document::Error>(Stream::new(dictionary, bytes))
        })
        .transpose()?;
    let mut dictionary = info_dictionary(document)?;
    for (key, value) in ["Title", "Author", "Subject", "Keywords"]
        .into_iter()
        .zip(fields.values())
    {
        if value.is_empty() {
            dictionary.remove(key.as_bytes());
        } else {
            dictionary.set(key, text_string(value));
        }
    }
    let info = document.add_object(dictionary);
    document.trailer.set("Info", info);
    if let Some(stream) = xmp {
        let metadata = document.add_object(stream);
        document.catalog_mut()?.set("Metadata", metadata);
    }
    Ok(())
}

fn property(namespace: &[u8], local: &[u8]) -> Option<usize> {
    match (namespace, local) {
        (DC, b"title") => Some(0),
        (DC, b"creator") => Some(1),
        (DC, b"description") => Some(2),
        (DC, b"subject") => Some(3),
        (PDF, b"Keywords") => Some(3),
        _ => None,
    }
}

fn namespace(value: ResolveResult<'_>) -> Result<Vec<u8>> {
    match value {
        ResolveResult::Bound(value) => Ok(value.as_ref().as_bytes().to_vec()),
        ResolveResult::Unbound => Ok(Vec::new()),
        ResolveResult::Unknown(_) => Err(invalid("XMP uses an undeclared XML namespace.")),
    }
}

fn canonical_description(fields: &Fields) -> String {
    let escape = quick_xml::escape::escape;
    format!(
        "<rdf:Description xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" rdf:about=\"\">\
         <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\
         <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\
         <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>\
         <pdf:Keywords>{}</pdf:Keywords><dc:subject><rdf:Bag><rdf:li>{}</rdf:li></rdf:Bag></dc:subject></rdf:Description>",
        escape(&fields.title),
        escape(&fields.author),
        escape(&fields.subject),
        escape(&fields.keywords),
        escape(&fields.keywords)
    )
}

struct Capture {
    index: usize,
    depth: usize,
    text: String,
    default_text: Option<String>,
    default_depth: Option<usize>,
}

fn transform_xmp(bytes: &[u8], replacement: Option<&Fields>) -> Result<(Vec<u8>, [String; 4])> {
    let content =
        std::str::from_utf8(bytes).map_err(|_| invalid("Only UTF-8 XMP packets are supported."))?;
    let mut reader = NsReader::from_str(content);
    reader.config_mut().expand_empty_elements = true;
    let mut writer = Writer::new(Vec::new());
    let mut stack: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut fields = [String::new(), String::new(), String::new(), String::new()];
    let mut capture: Option<Capture> = None;
    let mut rdf_count = 0;
    let mut root_count = 0;
    let mut skipped: Option<usize> = None;
    let malformed = || invalid("The XMP XML is malformed or uses an unsupported metadata layout.");
    for _ in 0..50_000 {
        let event = reader.read_event().map_err(|_| malformed())?;
        match event {
            Event::DocType(_) => return Err(invalid("XMP with DTDs/entities is not supported.")),
            Event::Decl(ref declaration) => {
                if let Some(encoding) = declaration.encoding()
                    && !encoding
                        .map_err(|_| malformed())?
                        .eq_ignore_ascii_case("UTF-8")
                {
                    return Err(invalid("Only UTF-8 XMP packets are supported."));
                }
            }
            Event::Start(ref start) => {
                if stack.len() >= 64 {
                    return Err(invalid("XMP exceeds the 64-level XML depth limit."));
                }
                if stack.is_empty() {
                    root_count += 1;
                }
                let (ns, local) = reader.resolver().resolve_element(start.name());
                let ns = namespace(ns)?;
                let local = local.as_ref().as_bytes().to_vec();
                if ns == RDF && local == b"RDF" {
                    rdf_count += 1;
                    if rdf_count > 1 {
                        return Err(malformed());
                    }
                }
                let description = ns == RDF && local == b"Description";
                let parent_description = stack
                    .last()
                    .is_some_and(|(ns, name)| ns == RDF && name == b"Description");
                if description {
                    if !stack
                        .last()
                        .is_some_and(|(ns, name)| ns == RDF && name == b"RDF")
                    {
                        return Err(malformed());
                    }
                    for attribute in start.attributes() {
                        let attribute = attribute.map_err(|_| malformed())?;
                        let (ns, name) = reader.resolver().resolve_attribute(attribute.key);
                        if namespace(ns)? == RDF
                            && (name.as_ref() == "nodeID"
                                || name.as_ref() == "about" && !attribute.value.is_empty())
                        {
                            return Err(invalid(
                                "XMP descriptions of named or non-document RDF subjects cannot be updated safely.",
                            ));
                        }
                    }
                }
                let field = property(&ns, &local);
                if let Some(index) = field {
                    if !parent_description || capture.is_some() {
                        return Err(malformed());
                    }
                    capture = Some(Capture {
                        index,
                        depth: stack.len(),
                        text: String::new(),
                        default_text: None,
                        default_depth: None,
                    });
                    if replacement.is_some() {
                        skipped = Some(stack.len());
                    }
                }
                let mut filtered = start.to_owned();
                filtered.clear_attributes();
                for (count, attribute) in start.attributes().enumerate() {
                    if count >= 256 {
                        return Err(invalid("XMP has too many XML attributes."));
                    }
                    let attribute = attribute.map_err(|_| malformed())?;
                    let (ns, name) = reader.resolver().resolve_attribute(attribute.key);
                    let ns = namespace(ns)?;
                    let field = property(&ns, name.as_ref().as_bytes());
                    if let Some(index) = field {
                        if !description {
                            return Err(malformed());
                        }
                        fields[index] = quick_xml::escape::unescape(&attribute.value)
                            .map_err(|_| malformed())?
                            .into_owned();
                        if replacement.is_some() {
                            continue;
                        }
                    }
                    if ns == b"http://www.w3.org/XML/1998/namespace"
                        && name.as_ref() == "lang"
                        && attribute.value.as_ref() == "x-default"
                        && let Some(capture) = &mut capture
                    {
                        capture.default_text = Some(String::new());
                        capture.default_depth = Some(stack.len());
                    }
                    filtered.push_attribute(attribute);
                }
                stack.push((ns, local));
                if skipped.is_none() {
                    writer.write_event(Event::Start(filtered))?;
                }
                continue;
            }
            Event::End(ref end) => {
                let depth = stack.len().checked_sub(1).ok_or_else(malformed)?;
                let (ns, local) = stack.last().ok_or_else(malformed)?;
                if ns == RDF
                    && local == b"RDF"
                    && let Some(fields) = replacement
                {
                    writer
                        .get_mut()
                        .extend_from_slice(canonical_description(fields).as_bytes());
                }
                if let Some(current) = &mut capture {
                    if current.default_depth == Some(depth) {
                        current.default_depth = None;
                    }
                    if current.index == 1 && ns == RDF && local == b"li" {
                        current.text.push_str("; ");
                    }
                    if current.depth == depth {
                        let current = capture.take().ok_or_else(malformed)?;
                        fields[current.index] = current
                            .default_text
                            .unwrap_or(current.text)
                            .trim()
                            .trim_end_matches("; ")
                            .to_owned();
                    }
                }
                stack.pop();
                if skipped == Some(depth) {
                    skipped = None;
                    continue;
                }
                if skipped.is_none() {
                    writer.write_event(Event::End(end.to_owned()))?;
                }
                continue;
            }
            Event::Eof => {
                if root_count != 1 || rdf_count != 1 || !stack.is_empty() {
                    return Err(malformed());
                }
                if fields.iter().any(|value| value.len() > MAX_FIELD_BYTES) {
                    return Err(invalid(
                        "An XMP descriptive field exceeds 4,096 UTF-8 bytes.",
                    ));
                }
                let output = writer.into_inner();
                if output.len() > MAX_XMP_BYTES {
                    return Err(invalid("The updated XMP exceeds 1 MiB."));
                }
                return Ok((output, fields));
            }
            _ => {}
        }
        let value = match &event {
            Event::Text(value) => Some(value.as_ref().to_owned()),
            Event::CData(value) => Some(value.as_ref().to_owned()),
            Event::GeneralRef(value) => {
                let name = value.as_ref();
                Some(
                    quick_xml::escape::unescape(&format!("&{name};"))
                        .map_err(|_| malformed())?
                        .into_owned(),
                )
            }
            _ => None,
        };
        if let (Some(value), Some(capture)) = (value, &mut capture) {
            capture.text.push_str(&value);
            if capture.default_depth.is_some()
                && let Some(default) = &mut capture.default_text
            {
                default.push_str(&value);
            }
        }
        if skipped.is_none() {
            writer.write_event(event)?;
        }
    }
    Err(invalid("XMP exceeds the 50,000-event XML limit."))
}
