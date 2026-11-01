use std::collections::{BTreeMap, HashSet};

use lopdf::{
    Document, Object, Stream,
    content::{Content, Operation},
    dictionary, text_string,
};

use super::{Candidate, CandidateKind, MAX_ITEMS, MAX_PAGES, Role};
use crate::{
    document::{Result, invalid},
    pdf_objects::{MAX_EDIT_CONTENT_BYTES, page_resources, resolve, resource},
};

#[derive(Clone)]
pub(super) struct Item {
    pub view: Candidate,
    pub start: usize,
    pub end: usize,
    pub managed: bool,
    pub ocr_stream: Option<lopdf::ObjectId>,
}

pub(super) struct Tag {
    pub item: Item,
    pub role: Role,
    pub mcid: Option<i64>,
}

fn authoring_features(document: &Document) -> Result<()> {
    let catalog = document.catalog()?;
    if catalog.has(b"StructTreeRoot") || catalog.has(b"MarkInfo") {
        return Err(invalid(
            "Existing or ambiguous logical structures are inspect-only. Folio never replaces an existing tag tree.",
        ));
    }
    if document.get_pages().len() > MAX_PAGES {
        return Err(invalid("Semantic authoring is limited to 100 pages."));
    }
    let mut pages = HashSet::new();
    for id in document.get_pages().values() {
        if !pages.insert(*id) {
            return Err(invalid(
                "Reused page objects cannot be given unambiguous semantic ownership.",
            ));
        }
        let page = document.get_dictionary(*id)?;
        if let Ok(value) = page.get(b"Annots")
            && !resolve(document, value)?.as_array()?.is_empty()
        {
            return Err(invalid(
                "Annotations and links need additional semantic associations. This document is inspect-only for this bounded authoring mode.",
            ));
        }
    }
    crate::preflight::walk(document, |object| {
        let dictionary = match object {
            Object::Dictionary(value) => value,
            Object::Stream(value) => &value.dict,
            _ => return Ok(()),
        };
        if [
            b"AcroForm".as_slice(),
            b"XFA",
            b"OCProperties",
            b"OC",
            b"AA",
            b"OpenAction",
            b"JavaScript",
            b"JS",
            b"A",
            b"StructParents",
            b"StructParent",
            b"RichMedia",
            b"Collection",
        ]
        .iter()
        .any(|key| dictionary.has(key))
        {
            return Err(invalid(
                "Forms, optional content, active actions/scripts or pre-existing structure ownership are not supported for semantic rewriting. Use an original untagged, noninteractive source.",
            ));
        }
        Ok(())
    })?;
    let metadata = crate::metadata::inspect(document)?;
    if !metadata.editable {
        return Err(invalid(metadata.reason.unwrap_or_else(|| {
            "Existing metadata cannot be updated safely.".into()
        })));
    }
    for finding in crate::preflight::inspect(document)? {
        if finding.category == "declarations"
            && finding.state == crate::preflight::State::Fact
            && (finding.property.starts_with("pdfaid:") || finding.property.starts_with("pdfuaid:"))
        {
            return Err(invalid(
                "This source already declares PDF/A or PDF/UA. This bounded tagged-copy mode does not rewrite conformance-declared documents or silently retain stale declarations.",
            ));
        }
    }
    Ok(())
}

pub(super) fn inventory(document: &Document) -> Result<Vec<Item>> {
    authoring_features(document)?;
    let mut items = Vec::new();
    let mut total_bytes = 0;
    let mut ocr = crate::ocr::canonical::Reader::default();
    for (page, id) in document.get_pages().values().enumerate() {
        let bytes = document.get_page_content_with_limit(*id, MAX_EDIT_CONTENT_BYTES)?;
        total_bytes += bytes.len();
        if total_bytes > 32 * 1024 * 1024 {
            return Err(invalid(
                "Semantic authoring exceeds 32 MiB of decoded page content.",
            ));
        }
        let content = Content::decode(&bytes)?;
        if content.operations.len() > 200_000 {
            return Err(invalid(
                "A semantic page exceeds 200,000 source operations.",
            ));
        }
        operation_spans(&bytes, &content.operations)?;
        let resources = page_resources(document, *id)?;
        let canonical = ocr.page(document, page)?;
        let mut prefix_end = 0;
        for span in &canonical {
            if span.start != prefix_end {
                return Err(invalid(
                    "Canonical OCR authoring requires complete pristine prefix streams before original page operators. Arbitrary marked text is not accepted.",
                ));
            }
            prefix_end = span.end + 1;
        }
        let analysis = crate::ocr::canonical::analysis_document(document, page, &canonical)?;
        let blocks = crate::paragraph::semantic_blocks(&analysis, page)?;
        let runs = crate::text_edit::page_runs(&analysis, page)?;
        if blocks.iter().any(|block| block.view.path.len() != 1) {
            return Err(invalid(format!(
                "Page {} contains nested Form text. Arbitrary nested Forms cannot be tagged safely in this bounded authoring mode.",
                page + 1
            )));
        }
        let mut index = 0;
        let mut depth = 0_usize;
        while let Some(operation) = content.operations.get(index) {
            if let Some(span) = canonical.iter().find(|span| span.start == index) {
                items.push(Item {
                    view: Candidate {
                        id: format!("page-{page}-ocr-{}-ops-{}-{}", span.stream.0, span.start, span.end),
                        page, kind: CandidateKind::Text, text: Some(span.text.clone()),
                        description: "Verified canonical invisible OCR text, in recorded source order; recognition accuracy is not verified".into(),
                    },
                    start: span.start, end: span.end, managed: false, ocr_stream: Some(span.stream),
                });
                if items.len() > MAX_ITEMS {
                    return Err(invalid(
                        "Semantic authoring is limited to 2,000 reviewed source candidates.",
                    ));
                }
                index = span.end + 1;
                continue;
            }
            let mut item = None;
            match operation.operator.as_str() {
                "q" => depth += 1,
                "Q" => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| invalid("Unbalanced graphics state in semantic source."))?;
                }
                "BT" => {
                    let start = index;
                    let end = content
                        .operations
                        .iter()
                        .enumerate()
                        .skip(index + 1)
                        .find(|(_, operation)| operation.operator == "ET")
                        .map(|(index, _)| index)
                        .ok_or_else(|| invalid("A source text object has no end."))?;
                    if content.operations[start + 1..end].iter().any(|operation| {
                        !matches!(
                            operation.operator.as_str(),
                            "Tc" | "Tw"
                                | "Tz"
                                | "TL"
                                | "Tf"
                                | "Tr"
                                | "Ts"
                                | "Td"
                                | "TD"
                                | "Tm"
                                | "T*"
                                | "Tj"
                                | "TJ"
                                | "'"
                                | "\""
                                | "g"
                                | "rg"
                                | "k"
                                | "G"
                                | "RG"
                                | "K"
                        )
                    }) {
                        return Err(invalid(
                            "A text object contains nested content, graphics or marked-content semantics that cannot be accounted for independently.",
                        ));
                    }
                    if let Some(block) = blocks.iter().find(|block| block.view.path == [start]) {
                        if block.end != end {
                            return Err(invalid("The source paragraph range is inconsistent."));
                        }
                        let logical = block.view.text.clone().ok_or_else(|| {
                            invalid(format!(
                                "Page {} contains unparsed text: {}",
                                page + 1,
                                block
                                    .view
                                    .reason
                                    .as_deref()
                                    .unwrap_or("unknown source font or layout")
                            ))
                        })?;
                        let mut decoded = String::new();
                        for run in runs.iter().filter(|run| {
                            run.path.len() == 1 && (start..=end).contains(&run.path[0])
                        }) {
                            decoded.push_str(run.text.as_deref().ok_or_else(|| invalid("A source text run cannot be decoded completely. It is not silently treated as decorative."))?);
                        }
                        if !logical
                            .chars()
                            .filter(|character| !character.is_whitespace())
                            .eq(decoded
                                .chars()
                                .filter(|character| !character.is_whitespace()))
                        {
                            return Err(invalid(format!(
                                "Page {} has source glyphs absent from its paragraph description, possibly because source metrics are unsupported. No partial source text is silently tagged.",
                                page + 1
                            )));
                        }
                        if !block.view.editable {
                            return Err(invalid(format!(
                                "Page {} has ambiguous or unsupported paragraph grouping: {}. Its internal reading order is not guessed.",
                                page + 1,
                                block
                                    .view
                                    .reason
                                    .as_deref()
                                    .unwrap_or("source layout could not be validated")
                            )));
                        }
                        if !logical.is_empty() {
                            item = Some((
                                CandidateKind::Text,
                                Some(logical),
                                "Source text object".to_owned(),
                                end,
                                false,
                            ));
                        }
                    } else if content.operations[start + 1..end].iter().any(|operation| {
                        fn contains_text(value: &Object) -> bool {
                            match value {
                                Object::String(value, _) => !value.is_empty(),
                                Object::Array(values) => values.iter().any(contains_text),
                                _ => false,
                            }
                        }
                        matches!(operation.operator.as_str(), "Tj" | "TJ" | "'" | "\"")
                            && operation.operands.iter().any(contains_text)
                    }) {
                        return Err(invalid(
                            "A source text object could not be decoded. It is not silently treated as decorative.",
                        ));
                    }
                    index = end;
                }
                "Do" => {
                    if operation.operands.len() != 1 {
                        return Err(invalid("An XObject invocation is malformed."));
                    }
                    let object = resource(
                        document,
                        &resources,
                        b"XObject",
                        operation.operands[0].as_name()?,
                    )?;
                    let stream = resolve(document, object)?.as_stream()?;
                    match stream.dict.get(b"Subtype")?.as_name()? {
                        b"Image" => {
                            item = Some((
                                CandidateKind::Figure,
                                None,
                                "Source image occurrence".into(),
                                index,
                                false,
                            ))
                        }
                        b"Form" if stream.dict.has(b"FolioParagraph") => {
                            let block = blocks
                                .iter()
                                .find(|block| block.view.path == [index] && block.managed)
                                .ok_or_else(|| {
                                    invalid(
                                        "A Folio paragraph failed its canonical source validation.",
                                    )
                                })?;
                            if stream.dict.has(b"Group")
                                || stream.dict.has(b"OC")
                                || stream.dict.has(b"Ref")
                                || stream.dict.has(b"OPI")
                            {
                                return Err(invalid(
                                    "A paragraph Form has unsupported transparency, optional-content or external-reference semantics.",
                                ));
                            }
                            item = Some((
                                CandidateKind::Text,
                                Some(block.view.text.clone().ok_or_else(|| {
                                    invalid("The paragraph has no validated logical source text.")
                                })?),
                                "Validated reflowed logical paragraph".into(),
                                index,
                                true,
                            ));
                        }
                        b"Form" => {
                            return Err(invalid(format!(
                                "Page {} contains an arbitrary Form XObject. Only validated top-level Folio paragraph Forms can be rewritten; nested/repeated custom Forms are inspect-only.",
                                page + 1
                            )));
                        }
                        _ => {
                            return Err(invalid(
                                "An unsupported XObject cannot be assigned safe semantic ownership.",
                            ));
                        }
                    }
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "sh" => {
                    item = Some((
                        CandidateKind::Graphic,
                        None,
                        format!("Painted graphic ({})", operation.operator),
                        index,
                        false,
                    ));
                }
                "BMC" | "BDC" | "EMC" | "MP" | "DP" => {
                    return Err(invalid(
                        "Pre-existing marked content is ambiguous without its original structure. It is inspect-only, not silently retagged.",
                    ));
                }
                "BI" | "ID" | "EI" => {
                    return Err(invalid(
                        "Inline images are not supported for safe semantic source binding.",
                    ));
                }
                "Tj" | "TJ" | "'" | "\"" | "ET" => {
                    return Err(invalid(
                        "A source text operation lies outside an accounted-for text object.",
                    ));
                }
                "cm" | "gs" | "m" | "l" | "c" | "v" | "y" | "h" | "re" | "n" | "W" | "W*" | "w"
                | "J" | "j" | "M" | "d" | "ri" | "i" | "CS" | "cs" | "SC" | "SCN" | "sc"
                | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" | "Tc" | "Tw" | "Tz" | "TL"
                | "Tf" | "Tr" | "Ts" => {}
                _ => {
                    return Err(invalid(format!(
                        "Unsupported semantic source operator {}. No source drawing is silently hidden.",
                        operation.operator
                    )));
                }
            }
            if let Some((kind, text, description, end, managed)) = item {
                let start = if operation.operator == "BT" {
                    *blocks
                        .iter()
                        .find(|block| block.end == end && !block.managed)
                        .expect("selected text")
                        .view
                        .path
                        .last()
                        .expect("text path")
                } else {
                    index
                };
                items.push(Item {
                    view: Candidate {
                        id: format!("page-{page}-ops-{start}-{end}"),
                        page,
                        kind,
                        text,
                        description,
                    },
                    start,
                    end,
                    managed,
                    ocr_stream: None,
                });
                if items.len() > MAX_ITEMS {
                    return Err(invalid(
                        "Semantic authoring is limited to 2,000 reviewed source candidates.",
                    ));
                }
            }
            index += 1;
        }
        if depth != 0 {
            return Err(invalid("Unbalanced graphics state in semantic source."));
        }
    }
    Ok(items)
}

fn marker(role: Role, mcid: Option<i64>, logical: Option<&str>) -> Result<Vec<u8>> {
    let operation = if let Some(mcid) = mcid {
        let mut properties = dictionary! { "MCID"=>mcid };
        if let Some(value) = logical {
            properties.set("ActualText", text_string(value));
        }
        Operation::new(
            "BDC",
            vec![
                Object::Name(role.name().as_bytes().to_vec()),
                Object::Dictionary(properties),
            ],
        )
    } else {
        Operation::new("BMC", vec![Object::Name(b"Artifact".to_vec())])
    };
    let mut bytes = Content {
        operations: vec![operation],
    }
    .encode()?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn tag_pages(document: &mut Document, tags: &[Tag]) -> Result<()> {
    let pages: Vec<_> = document.get_pages().values().copied().collect();
    for (page, id) in pages.into_iter().enumerate() {
        let bytes = document.get_page_content_with_limit(id, MAX_EDIT_CONTENT_BYTES)?;
        let operations = Content::decode(&bytes)?.operations;
        let spans = operation_spans(&bytes, &operations)?;
        let resources = page_resources(document, id)?;
        let mut edits = BTreeMap::new();
        for tag in tags.iter().filter(|tag| tag.item.view.page == page) {
            let start = spans[tag.item.start].0;
            let end = spans[tag.item.end].1;
            let body = &bytes[start..end];
            // The page's exact Do occurrence owns the semantics; the canonical Form stays unmarked.
            let logical = if tag.item.managed || tag.item.ocr_stream.is_some() {
                tag.item.view.text.as_deref()
            } else {
                None
            };
            let prefix = marker(tag.role, tag.mcid, logical)?;
            edits.insert(start, (end, prefix, body, tag.item.ocr_stream));
        }
        let mut marked = Vec::with_capacity(bytes.len() + edits.len() * 64);
        let mut contents = Vec::new();
        let mut cursor = 0;
        for (start, (end, prefix, body, ocr_stream)) in edits {
            if start < cursor {
                return Err(invalid("Semantic source ranges overlap."));
            }
            marked.extend_from_slice(&bytes[cursor..start]);
            marked.extend_from_slice(b"\n");
            marked.extend_from_slice(&prefix);
            if let Some(original) = ocr_stream {
                let mut stream = Stream::new(dictionary! {}, std::mem::take(&mut marked));
                stream.compress()?;
                contents.push(Object::Reference(document.add_object(stream)));
                contents.push(Object::Reference(original));
            } else {
                marked.extend_from_slice(body);
            }
            marked.extend_from_slice(b"\nEMC\n");
            cursor = end;
        }
        marked.extend_from_slice(&bytes[cursor..]);
        let mut stream = Stream::new(dictionary! {}, marked);
        stream.compress()?;
        contents.push(Object::Reference(document.add_object(stream)));
        let dictionary = document.get_dictionary_mut(id)?;
        dictionary.set("Contents", contents);
        dictionary.set("Resources", resources);
        if tags
            .iter()
            .any(|tag| tag.item.view.page == page && tag.role != Role::Artifact)
        {
            dictionary.set("StructParents", page as i64);
        }
        dictionary.set("Tabs", "S");
    }
    Ok(())
}

pub(super) fn operation_spans(
    bytes: &[u8],
    operations: &[Operation],
) -> Result<Vec<(usize, usize)>> {
    // Keep every original drawing operand byte-for-byte; only insert semantic delimiters.
    let mut spans = Vec::new();
    let (mut index, mut start) = (0, 0);
    let mut containers = Vec::new();
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() || byte == 0 {
            index += 1;
            continue;
        }
        if byte == b'%' {
            while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                index += 1;
            }
            continue;
        }
        if byte == b'(' {
            index += 1;
            let mut depth = 1;
            while index < bytes.len() && depth > 0 {
                match bytes[index] {
                    b'\\' => {
                        index += 1;
                        if index < bytes.len()
                            && bytes[index] == b'\r'
                            && bytes.get(index + 1) == Some(&b'\n')
                        {
                            index += 1;
                        }
                    }
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                index += 1;
            }
            if depth != 0 {
                return Err(invalid("Unterminated source string in semantic binding."));
            }
            continue;
        }
        if byte == b'<' {
            if bytes.get(index + 1) == Some(&b'<') {
                containers.push(b'>');
                index += 2;
            } else {
                index += 1;
                while index < bytes.len() && bytes[index] != b'>' {
                    index += 1;
                }
                if index == bytes.len() {
                    return Err(invalid("Unterminated source hex string."));
                }
                index += 1;
            }
            continue;
        }
        if byte == b'[' {
            containers.push(b']');
            index += 1;
            continue;
        }
        if byte == b']' || byte == b'>' {
            let closing = if byte == b'>' {
                if bytes.get(index + 1) != Some(&b'>') {
                    return Err(invalid("Malformed source dictionary delimiter."));
                }
                index += 1;
                b'>'
            } else {
                b']'
            };
            if containers.pop() != Some(closing) {
                return Err(invalid("Unbalanced source operands."));
            }
            index += 1;
            continue;
        }
        let name = byte == b'/';
        let token_start = index;
        if name {
            index += 1;
        }
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && !matches!(
                bytes[index],
                0 | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%'
            )
        {
            index += 1;
        }
        if index == token_start {
            return Err(invalid("Unsupported source token in semantic binding."));
        }
        if name || !containers.is_empty() {
            continue;
        }
        let token = &bytes[token_start..index];
        let operand = matches!(token, b"true" | b"false" | b"null")
            || token
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.'));
        if operand {
            continue;
        }
        let expected = operations
            .get(spans.len())
            .ok_or_else(|| invalid("Source operator ranges disagree with the decoded stream."))?;
        if token != expected.operator.as_bytes() || matches!(token, b"BI" | b"ID" | b"EI") {
            return Err(invalid(
                "Source operator ranges are ambiguous or contain an inline image. No tagged copy was made.",
            ));
        }
        spans.push((start, index));
        start = index;
    }
    if !containers.is_empty() || spans.len() != operations.len() {
        return Err(invalid("Incomplete source operation ranges."));
    }
    Ok(spans)
}
