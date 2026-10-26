use std::{collections::HashSet, path::Path};

use lopdf::{Dictionary, Document, Object};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use serde::Serialize;

use crate::{
    archive::SourceRef,
    document::{Result, inherited, invalid},
    page_geometry::{PageGeometry, read_box},
    pdf_objects::{resolve, stream_bytes, text},
    workspace::Workspace,
};

pub const MAX_PAGES: usize = 500;
pub const MAX_OBJECTS: usize = 200_000;
pub const MAX_GRAPH_NODES: usize = 50_000;
pub const MAX_DEPTH: usize = 64;
pub const MAX_FINDINGS: usize = 4000;
pub const MAX_REPORT_BYTES: usize = 2 * 1024 * 1024;
const MAX_XMP_BYTES: usize = 1024 * 1024;
pub const SCOPE: &str = "Local technical observations of the current working PDF: reachable dictionaries/resource declarations, font-program stream presence (not font validity or actual usage), image dictionary properties (not decoded quality), page geometry, form/active-feature keys, catalog language/tag presence, and namespace-resolved UTF-8 XMP PDF/A/PDF/UA declarations. Absence is limited to this inspected scope. No content-stream interpretation, visual/accessibility audit, tag/reading-order validation, PDF/UA or arbitrary-input PDF/A validation, signature/trust check, unreachable-object/history inspection, or remediation.";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Fact,
    Absent,
    Unsupported,
    Unchecked,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub category: String,
    pub property: String,
    pub state: State,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema: &'static str,
    pub schema_version: u8,
    pub inspection_id: u64,
    #[serde(flatten)]
    pub source: SourceRef,
    pub epoch: u64,
    pub name: String,
    pub password_protected: bool,
    pub scope: &'static str,
    pub input_conformance: &'static str,
    pub findings: Vec<Finding>,
    pub suggested_name: String,
}

#[derive(Default)]
pub(crate) struct Inspections {
    next_id: u64,
    report: Option<Report>,
}

impl Inspections {
    pub(crate) fn invalidate(&mut self, id: u64) {
        if self
            .report
            .as_ref()
            .is_some_and(|report| report.source.document_id == id)
        {
            self.report = None;
        }
    }
}

impl Workspace {
    pub fn inspect_technical(&mut self, source: SourceRef) -> Result<Report> {
        let session = self.ensure_technical_ready(source.document_id, source.revision)?;
        let inspection_id = self
            .inspections
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Inspection identifiers are exhausted. Restart Folio."))?;
        let findings = session.technical_findings()?;
        let info = session.info()?;
        let report = Report {
            schema: "folio-technical-preflight",
            schema_version: 1,
            inspection_id,
            source,
            epoch: self.technical_epoch(info.id),
            name: info.name.clone(),
            password_protected: info.password_protected,
            scope: SCOPE,
            input_conformance: "NOT VALIDATED. Declarations and tag presence are observations, not proof of conformance.",
            findings,
            suggested_name: format!(
                "{}-technical-preflight.json",
                crate::output::safe_stem(&info.name)
            ),
        };
        bounded_report(&report)?;
        self.inspections.next_id = inspection_id;
        self.inspections.report = Some(report.clone());
        Ok(report)
    }

    pub fn check_inspection(&self, inspection_id: u64) -> Result<Report> {
        let report = self.inspections.report.as_ref().filter(|report| report.inspection_id == inspection_id)
            .ok_or_else(|| invalid("This technical report was replaced, canceled or invalidated by a source edit, draft, staged image or closed tab. Inspect the current PDF again."))?;
        self.ensure_technical_ready(report.source.document_id, report.source.revision)?;
        if report.epoch != self.technical_epoch(report.source.document_id) {
            return Err(invalid(
                "The inspection source epoch changed. Inspect the current PDF again.",
            ));
        }
        Ok(report.clone())
    }

    pub fn inspection_report(
        &self,
        inspection_id: u64,
        allow_unencrypted: bool,
    ) -> Result<Vec<u8>> {
        let report = self.check_inspection(inspection_id)?;
        if report.password_protected && !allow_unencrypted {
            return Err(invalid(
                "Acknowledge that this technical report discloses unencrypted document properties and does not retain PDF passwords, permissions or signatures.",
            ));
        }
        bounded_report(&report)
    }

    pub fn write_inspection_report(
        &self,
        inspection_id: u64,
        allow_unencrypted: bool,
        path: &Path,
    ) -> Result<usize> {
        let report = self.check_inspection(inspection_id)?;
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case("json"))
        {
            return Err(invalid("Use a .json filename for the technical report."));
        }
        self.ensure_output_available(report.source.document_id, path)?;
        self.ensure_technical_ready(report.source.document_id, report.source.revision)?
            .ensure_separate_output(path)?;
        let bytes = self.inspection_report(inspection_id, allow_unencrypted)?;
        crate::output::write_new_copy(path, &bytes)?;
        Ok(bytes.len())
    }

    pub fn cancel_inspection(&mut self, inspection_id: u64) -> bool {
        if self
            .inspections
            .report
            .as_ref()
            .is_some_and(|report| report.inspection_id == inspection_id)
        {
            self.inspections.report = None;
            true
        } else {
            false
        }
    }
}

fn bounded_report(report: &Report) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(report).map_err(|error| {
        invalid(format!(
            "The technical report could not be encoded: {error}"
        ))
    })?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err(invalid(
            "The technical report exceeds 2 MiB. No partial report is exported.",
        ));
    }
    Ok(bytes)
}

pub(crate) fn walk(
    document: &Document,
    mut visit: impl FnMut(&Object) -> Result<()>,
) -> Result<()> {
    if document.objects.len() > MAX_OBJECTS {
        return Err(invalid(
            "Technical inspection supports at most 200,000 PDF objects.",
        ));
    }
    let root = document.trailer.get(b"Root")?;
    let mut stack = vec![(root, 0)];
    let mut seen = HashSet::new();
    let mut nodes = 0;
    while let Some((object, depth)) = stack.pop() {
        nodes += 1;
        if nodes > MAX_GRAPH_NODES || depth > MAX_DEPTH {
            return Err(invalid(
                "The reachable PDF graph exceeds 50,000 nodes or 64 levels. No partial technical result or archival copy is produced.",
            ));
        }
        if let Object::Reference(id) = object {
            // Resolve chains separately: a cyclic reference chain is malformed,
            // unlike normal page-tree/resource graph backreferences.
            let value = resolve(document, object)?;
            if seen.insert(*id) {
                stack.push((value, depth));
            }
            continue;
        }
        visit(object)?;
        match object {
            Object::Dictionary(dictionary) => {
                if stack.len() + dictionary.len() > MAX_GRAPH_NODES {
                    return Err(invalid(
                        "The PDF dictionary graph exceeds 50,000 queued nodes.",
                    ));
                }
                stack.extend(dictionary.iter().map(|(_, value)| (value, depth + 1)));
            }
            Object::Stream(stream) => {
                if stack.len() + stream.dict.len() > MAX_GRAPH_NODES {
                    return Err(invalid("The PDF stream graph exceeds 50,000 queued nodes."));
                }
                stack.extend(stream.dict.iter().map(|(_, value)| (value, depth + 1)));
            }
            Object::Array(values) => {
                if stack.len() + values.len() > MAX_GRAPH_NODES {
                    return Err(invalid("The PDF array graph exceeds 50,000 queued nodes."));
                }
                stack.extend(values.iter().map(|value| (value, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(())
}

struct Findings(Vec<Finding>);

impl Findings {
    fn add(
        &mut self,
        category: &str,
        property: impl Into<String>,
        state: State,
        detail: impl Into<String>,
        page: Option<usize>,
    ) -> Result<()> {
        if self.0.len() >= MAX_FINDINGS {
            return Err(invalid(
                "The technical report exceeds 4,000 findings. No partial report is exported.",
            ));
        }
        let detail = detail.into();
        if detail.len() > 8192 {
            return Err(invalid(
                "A technical property exceeds the 8,192-byte finding limit.",
            ));
        }
        self.0.push(Finding {
            category: category.into(),
            property: property.into(),
            state,
            detail,
            page,
        });
        Ok(())
    }
    fn observed(
        &mut self,
        category: &str,
        property: impl Into<String>,
        value: Result<String>,
        page: Option<usize>,
    ) -> Result<()> {
        match value {
            Ok(value) => self.add(category, property, State::Fact, value, page),
            Err(error) => self.add(
                category,
                property,
                State::Unsupported,
                error.to_string(),
                page,
            ),
        }
    }
}

pub(crate) fn inspect(document: &Document) -> Result<Vec<Finding>> {
    if document.objects.len() > MAX_OBJECTS {
        return Err(invalid(
            "Technical inspection supports at most 200,000 PDF objects.",
        ));
    }
    let pages = document.get_pages();
    if pages.is_empty() || pages.len() > MAX_PAGES {
        return Err(invalid(
            "Technical inspection supports 1 to 500 pages; no partial page scan is reported.",
        ));
    }
    let mut findings = Findings(Vec::new());
    findings.add(
        "document",
        "PDF header",
        State::Fact,
        format!(
            "PDF {}; {} pages; {} parsed objects.",
            document.version,
            pages.len(),
            document.objects.len()
        ),
        None,
    )?;
    let catalog = document.catalog()?;
    if let Ok(language) = catalog.get(b"Lang") {
        findings.observed(
            "structure",
            "Document language",
            text(document, language, 4096)
                .map(|value| format!("Catalog /Lang: {value}. Language correctness is unchecked.")),
            None,
        )?;
    } else {
        findings.add(
            "structure",
            "Document language",
            State::Absent,
            "No catalog /Lang entry.",
            None,
        )?;
    }
    for (entry, label) in [
        (b"StructTreeRoot".as_slice(), "Structure tree"),
        (b"AcroForm", "AcroForm"),
        (b"Metadata", "XMP packet"),
    ] {
        findings.add(
            "structure",
            label,
            if catalog.has(entry) {
                State::Fact
            } else {
                State::Absent
            },
            if catalog.has(entry) {
                format!(
                    "Catalog /{} entry present; not a validation of its semantics.",
                    String::from_utf8_lossy(entry)
                )
            } else {
                format!("No catalog /{} entry.", String::from_utf8_lossy(entry))
            },
            None,
        )?;
    }
    match (|| -> Result<Option<String>> {
        match catalog.get(b"MarkInfo") {
            Ok(value) => {
                let dictionary = resolve(document, value)?.as_dict()?;
                match dictionary.get(b"Marked") {
                    Ok(value) => Ok(Some(format!(
                        "MarkInfo /Marked = {}. This is only a flag, not verified tagging.",
                        resolve(document, value)?.as_bool()?
                    ))),
                    Err(lopdf::Error::DictKey(_)) => Ok(None),
                    Err(error) => Err(error.into()),
                }
            }
            Err(lopdf::Error::DictKey(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    })() {
        Ok(Some(detail)) => findings.add("structure", "Marked flag", State::Fact, detail, None)?,
        Ok(None) => findings.add(
            "structure",
            "Marked flag",
            State::Absent,
            "No MarkInfo /Marked flag declared. Tag correctness is unchecked.",
            None,
        )?,
        Err(error) => findings.add(
            "structure",
            "Marked flag",
            State::Unsupported,
            error.to_string(),
            None,
        )?,
    }
    for (number, id) in pages {
        let page = Some(number as usize);
        findings.observed("pages", format!("Page {number} geometry"), PageGeometry::for_page(document, id).map(|geometry| format!(
            "MediaBox {:?}; effective CropBox {:?}; rotation {} degrees; page-local UserUnit {}; visible physical size {} x {} pt.",
            geometry.media_box, geometry.bounds, geometry.rotation, geometry.user_unit, geometry.width_points, geometry.height_points
        )), page)?;
        if inherited(document, id, b"CropBox")?.is_none() {
            findings.add(
                "pages",
                format!("Page {number} explicit CropBox"),
                State::Absent,
                "No inherited/page CropBox; MediaBox is the effective crop.",
                page,
            )?;
        }
        let dictionary = document.get_dictionary(id)?;
        for entry in [b"BleedBox".as_slice(), b"TrimBox", b"ArtBox"] {
            let name = String::from_utf8_lossy(entry);
            match dictionary.get(entry) {
                Ok(value) => findings.observed(
                    "pages",
                    format!("Page {number} {name}"),
                    read_box(document, value)
                        .map(|bounds| format!("{bounds:?} in default user units.")),
                    page,
                )?,
                Err(lopdf::Error::DictKey(_)) => findings.add(
                    "pages",
                    format!("Page {number} {name}"),
                    State::Absent,
                    "No page-local entry (PDF default applies).",
                    page,
                )?,
                Err(error) => return Err(error.into()),
            }
        }
    }
    let mut fonts = 0;
    let mut images = 0;
    let mut features = HashSet::new();
    walk(document, |object| {
        let dictionary = match object {
            Object::Dictionary(value) => value,
            Object::Stream(value) => &value.dict,
            _ => return Ok(()),
        };
        if optional_name(document, dictionary, b"Type")? == Some(b"Font") {
            fonts += 1;
            let property = format!("Reachable font dictionary {fonts}");
            match font_description(document, dictionary) {
                Ok((embedded, detail)) => findings.add(
                    "fonts",
                    property,
                    if embedded { State::Fact } else { State::Absent },
                    detail,
                    None,
                )?,
                Err(error) => findings.add(
                    "fonts",
                    property,
                    State::Unsupported,
                    error.to_string(),
                    None,
                )?,
            }
        }
        let subtype = optional_name(document, dictionary, b"Subtype")?;
        if subtype == Some(b"Image") {
            images += 1;
            findings.observed(
                "images",
                format!("Reachable image dictionary {images}"),
                image_description(document, dictionary),
                None,
            )?;
        }
        for entry in [
            b"XFA".as_slice(),
            b"OpenAction",
            b"AA",
            b"JS",
            b"JavaScript",
            b"EmbeddedFiles",
            b"AF",
            b"OCProperties",
            b"Sig",
            b"Collection",
        ] {
            if dictionary.has(entry) {
                features.insert(format!("/{} key", String::from_utf8_lossy(entry)));
            }
        }
        if let Some(action) = optional_name(document, dictionary, b"S")?
            && matches!(
                action,
                b"JavaScript"
                    | b"Launch"
                    | b"SubmitForm"
                    | b"ImportData"
                    | b"Rendition"
                    | b"GoToR"
                    | b"URI"
            )
        {
            features.insert(format!("/S /{} action", String::from_utf8_lossy(action)));
        }
        if matches!(
            subtype,
            Some(b"Widget" | b"RichMedia" | b"3D" | b"Movie" | b"Sound" | b"Screen")
        ) || optional_name(document, dictionary, b"Type")? == Some(b"EmbeddedFile")
        {
            features.insert(format!(
                "{} object",
                subtype
                    .map(|name| format!("/{}", String::from_utf8_lossy(name)))
                    .unwrap_or("/EmbeddedFile".into())
            ));
        }
        Ok(())
    })?;
    if fonts == 0 {
        findings.add("fonts", "Font dictionaries", State::Absent, "No /Type /Font dictionaries in the bounded reachable graph. This does not prove text is absent.", None)?;
    }
    if images == 0 {
        findings.add("images", "Image XObjects", State::Absent, "No /Subtype /Image dictionaries in the bounded reachable graph. Inline content-stream images were not inspected.", None)?;
    }
    let mut features: Vec<_> = features.into_iter().collect();
    features.sort();
    if features.is_empty() {
        findings.add("features", "Inspected active/attachment keys", State::Absent, "No inspected action/widget/attachment/multimedia keys found in the reachable graph. Content streams and unreachable/history objects were not inspected.", None)?;
    } else {
        for feature in features {
            findings.add("features", feature, State::Fact, "Present in the reachable dictionary graph. Not executed; behavior and appearance are unchecked.", None)?;
        }
    }
    match catalog.get(b"Metadata") {
        Ok(value) => match (|| declarations(&stream_bytes(resolve(document, value)?.as_stream()?, MAX_XMP_BYTES)?))() {
            Ok(values) if values.is_empty() => findings.add("declarations", "PDF/A / PDF/UA", State::Absent, "No PDF/A or PDF/UA identification properties in the parsed UTF-8 XMP packet. Input conformance is NOT validated.", None)?,
            Ok(values) => for (name, value) in values { findings.add("declarations", name, State::Fact, format!("Declared value: {value}. UNVERIFIED declaration, not validated conformance."), None)?; },
            Err(error) => findings.add("declarations", "PDF/A / PDF/UA XMP", State::Unsupported, error.to_string(), None)?,
        },
        Err(lopdf::Error::DictKey(_)) => findings.add("declarations", "PDF/A / PDF/UA", State::Absent, "No catalog XMP packet; no identification declaration observed. This is not a compliance conclusion.", None)?,
        Err(error) => return Err(error.into()),
    }
    for property in [
        "PDF/A conformance",
        "PDF/UA / accessibility / reading order",
        "Font program validity and content-stream usage",
        "Inline images / decoded image quality",
        "Signatures / trust / hidden history",
    ] {
        findings.add(
            "boundaries",
            property,
            State::Unchecked,
            "Outside this technical inspection. No pass/fail or remediation claim is made.",
            None,
        )?;
    }
    Ok(findings.0)
}

fn optional_name<'a>(
    document: &'a Document,
    dictionary: &'a Dictionary,
    entry: &[u8],
) -> Result<Option<&'a [u8]>> {
    match dictionary.get(entry) {
        Ok(value) => Ok(Some(resolve(document, value)?.as_name()?)),
        Err(lopdf::Error::DictKey(_)) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn font_description(document: &Document, dictionary: &Dictionary) -> Result<(bool, String)> {
    let subtype = optional_name(document, dictionary, b"Subtype")?
        .ok_or_else(|| invalid("The font has no subtype."))?;
    let name = optional_name(document, dictionary, b"BaseFont")?
        .map(String::from_utf8_lossy)
        .unwrap_or("(no BaseFont)".into());
    if name.len() > 4096 {
        return Err(invalid("The font name exceeds 4,096 bytes."));
    }
    if subtype == b"Type3" {
        return Err(invalid(format!(
            "{name}: Type 3 glyph-program embedding is not inspected."
        )));
    }
    let descriptor = if subtype == b"Type0" {
        let descendants = resolve(document, dictionary.get(b"DescendantFonts")?)?.as_array()?;
        if descendants.len() != 1 {
            return Err(invalid(
                "A Type 0 font must have one inspectable descendant.",
            ));
        }
        resolve(document, &descendants[0])?.as_dict()?
    } else {
        dictionary
    };
    let Some(value) = descriptor.get(b"FontDescriptor").ok() else {
        return Ok((
            false,
            format!(
                "{name}, subtype /{}: no FontDescriptor or embedded font-program stream observed.",
                String::from_utf8_lossy(subtype)
            ),
        ));
    };
    let descriptor = resolve(document, value)?.as_dict()?;
    let mut programs = Vec::new();
    for entry in [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"] {
        if let Ok(value) = descriptor.get(entry) {
            let stream = resolve(document, value)?.as_stream()?;
            if stream.content.is_empty() {
                return Err(invalid("An embedded font-program stream is empty."));
            }
            programs.push(format!(
                "/{} ({} encoded bytes)",
                String::from_utf8_lossy(entry),
                stream.content.len()
            ));
        }
    }
    Ok((
        !programs.is_empty(),
        format!(
            "{name}, subtype /{}: {}. Program validity/coverage and actual text use are unchecked.",
            String::from_utf8_lossy(subtype),
            if programs.is_empty() {
                "no embedded font-program stream".into()
            } else {
                format!("embedded stream(s) present: {}", programs.join(", "))
            }
        ),
    ))
}

fn image_description(document: &Document, dictionary: &Dictionary) -> Result<String> {
    let width = resolve(document, dictionary.get(b"Width")?)?.as_i64()?;
    let height = resolve(document, dictionary.get(b"Height")?)?.as_i64()?;
    if width <= 0 || height <= 0 {
        return Err(invalid("An image has nonpositive dimensions."));
    }
    let bits = dictionary
        .get(b"BitsPerComponent")
        .ok()
        .map(|value| resolve(document, value).and_then(|value| Ok(value.as_i64()?)))
        .transpose()?;
    let color = match dictionary.get(b"ColorSpace") {
        Ok(value) => match resolve(document, value)? {
            Object::Name(name) if name.len() < 256 => format!("/{}", String::from_utf8_lossy(name)),
            Object::Array(values) if !values.is_empty() => format!(
                "array family /{}",
                String::from_utf8_lossy(resolve(document, &values[0])?.as_name()?)
            ),
            _ => return Err(invalid("Unsupported image color-space declaration.")),
        },
        Err(lopdf::Error::DictKey(_)) => {
            "(no explicit color space; may be a mask/codec-defined)".into()
        }
        Err(error) => return Err(error.into()),
    };
    Ok(format!(
        "{width} x {height} declared pixels; BitsPerComponent {bits:?}; color space {color}; mask present {}. Pixels, placement DPI and codec validity unchecked.",
        dictionary.has(b"SMask") || dictionary.has(b"Mask")
    ))
}

fn declaration_name(ns: &[u8], local: &[u8]) -> Option<String> {
    match ns {
        b"http://www.aiim.org/pdfa/ns/id/"
            if matches!(local, b"part" | b"conformance" | b"rev" | b"amd") =>
        {
            Some(format!("pdfaid:{}", String::from_utf8_lossy(local)))
        }
        b"http://www.aiim.org/pdfua/ns/id/" if matches!(local, b"part" | b"rev" | b"amd") => {
            Some(format!("pdfuaid:{}", String::from_utf8_lossy(local)))
        }
        _ => None,
    }
}

fn namespace(value: ResolveResult<'_>) -> Result<Vec<u8>> {
    match value {
        ResolveResult::Bound(value) => Ok(value.as_ref().as_bytes().to_vec()),
        ResolveResult::Unbound => Ok(Vec::new()),
        ResolveResult::Unknown(_) => Err(invalid("XMP uses an undeclared namespace.")),
    }
}

fn declarations(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let content = std::str::from_utf8(bytes)
        .map_err(|_| invalid("Only bounded UTF-8 XMP declarations are inspected."))?;
    let mut reader = NsReader::from_str(content);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0;
    let mut roots = 0;
    let mut rdf = 0;
    let mut values = Vec::new();
    let mut capture: Option<(String, String, usize)> = None;
    for _ in 0..50_000 {
        match reader
            .read_event()
            .map_err(|error| invalid(format!("Malformed XMP: {error}")))?
        {
            Event::DocType(_) => return Err(invalid("DTD/entity-bearing XMP is unsupported.")),
            Event::Decl(decl) => {
                if let Some(encoding) = decl.encoding()
                    && !encoding
                        .map_err(|error| invalid(error.to_string()))?
                        .eq_ignore_ascii_case("UTF-8")
                {
                    return Err(invalid("Only UTF-8 XMP declarations are inspected."));
                }
            }
            Event::Start(start) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
                if depth > 64 {
                    return Err(invalid("XMP exceeds 64 XML levels."));
                }
                let (ns, local) = reader.resolver().resolve_element(start.name());
                let ns = namespace(ns)?;
                if ns == b"http://www.w3.org/1999/02/22-rdf-syntax-ns#" && local.as_ref() == "RDF" {
                    rdf += 1;
                }
                if let Some(name) = declaration_name(&ns, local.as_ref().as_bytes()) {
                    if capture.is_some() {
                        return Err(invalid(
                            "Nested identification declarations are unsupported.",
                        ));
                    }
                    capture = Some((name, String::new(), depth));
                }
                for (index, attribute) in start.attributes().enumerate() {
                    if index >= 256 {
                        return Err(invalid("XMP exceeds 256 attributes per element."));
                    }
                    let attribute = attribute.map_err(|error| invalid(error.to_string()))?;
                    let (ns, local) = reader.resolver().resolve_attribute(attribute.key);
                    if let Some(name) = declaration_name(&namespace(ns)?, local.as_ref().as_bytes())
                    {
                        values.push((
                            name,
                            quick_xml::escape::unescape(&attribute.value)
                                .map_err(|error| invalid(error.to_string()))?
                                .into_owned(),
                        ));
                    }
                }
            }
            Event::Text(value) => {
                if let Some((_, text, _)) = &mut capture {
                    text.push_str(value.as_ref());
                }
            }
            Event::CData(value) => {
                if let Some((_, text, _)) = &mut capture {
                    text.push_str(value.as_ref());
                }
            }
            Event::GeneralRef(value) => {
                let escaped = format!("&{};", value.as_ref());
                let value = quick_xml::escape::unescape(&escaped)
                    .map_err(|error| invalid(error.to_string()))?;
                if let Some((_, text, _)) = &mut capture {
                    text.push_str(&value);
                }
            }
            Event::End(_) => {
                if capture
                    .as_ref()
                    .is_some_and(|(_, _, start)| *start == depth)
                {
                    let (name, value, _) = capture.take().expect("capture checked");
                    values.push((name, value.trim().into()));
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("Malformed XMP nesting."))?;
            }
            Event::Eof => {
                if depth != 0 || roots != 1 || rdf != 1 || capture.is_some() {
                    return Err(invalid("Malformed or unsupported XMP RDF layout."));
                }
                return Ok(values);
            }
            _ => {}
        }
        if values.len() > 64
            || values.iter().any(|(_, value)| value.len() > 4096)
            || capture
                .as_ref()
                .is_some_and(|(_, value, _)| value.len() > 4096)
        {
            return Err(invalid(
                "XMP identification exceeds 64 declarations or 4,096 bytes per value.",
            ));
        }
    }
    Err(invalid("XMP exceeds 50,000 XML events."))
}
