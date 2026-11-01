use std::collections::{BTreeMap, HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, content::Content};

use super::{ContentBinding, MAX_ITEMS, MAX_PAGES, SemanticNode};
use crate::{
    document::{Result, invalid},
    pdf_objects::{
        MAX_EDIT_CONTENT_BYTES, form_resources, key, page_resources, resolve, resource, text,
    },
};

#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct Scope {
    page: usize,
    stream: Option<ObjectId>,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Binding {
    scope: Scope,
    mcid: i64,
}
#[derive(Default)]
struct Mark {
    text: String,
    actual: Option<String>,
    painted: bool,
}
#[derive(Clone)]
struct Context {
    binding: Option<Binding>,
    artifact: bool,
}

#[derive(Default)]
struct ContentMap {
    marks: HashMap<Binding, Mark>,
    scopes: BTreeMap<Scope, i64>,
    owners: BTreeMap<i64, Scope>,
    forms: HashSet<ObjectId>,
    visiting: HashSet<ObjectId>,
    bytes: usize,
    operations: usize,
    unbound: usize,
    ocr: HashMap<usize, Vec<crate::ocr::canonical::CanonicalText>>,
}

fn integer(document: &Document, object: &Object) -> Result<i64> {
    let value = resolve(document, object)?.as_i64()?;
    if !(0..=200_000).contains(&value) {
        return Err(invalid(
            "A structure key or MCID is negative or exceeds the supported 200,000 limit.",
        ));
    }
    Ok(value)
}

impl ContentMap {
    fn register(
        &mut self,
        document: &Document,
        scope: &Scope,
        dictionary: &Dictionary,
    ) -> Result<()> {
        if let Ok(value) = dictionary.get(b"StructParents") {
            let value = integer(document, value)?;
            if self.scopes.insert(scope.clone(), value).is_some()
                || self.owners.insert(value, scope.clone()).is_some()
            {
                return Err(invalid(
                    "StructParents ownership is shared or duplicated across page/Form scopes.",
                ));
            }
        }
        Ok(())
    }

    fn meaningful(&mut self, context: &Context, value: Option<&str>, painted: bool) -> Result<()> {
        if context.artifact {
            return Ok(());
        }
        if let Some(binding) = &context.binding {
            let mark = self
                .marks
                .get_mut(binding)
                .ok_or_else(|| invalid("A marked-content binding is missing."))?;
            if let Some(value) = value {
                mark.text.push_str(value);
                if mark.text.len() > super::MAX_TEXT_BYTES {
                    return Err(invalid("Bound semantic text exceeds 2 MiB."));
                }
            }
            mark.painted |= painted;
        } else {
            self.unbound += 1;
        }
        Ok(())
    }

    fn scan(
        &mut self,
        document: &Document,
        bytes: Vec<u8>,
        resources: &Dictionary,
        (scope, prefix): (Scope, &[usize]),
        mut context: Context,
        runs: &HashMap<Vec<usize>, Option<String>>,
    ) -> Result<()> {
        self.bytes += bytes.len();
        if self.bytes > 32 * 1024 * 1024 || prefix.len() > 16 {
            return Err(invalid(
                "Structure content inspection exceeds 32 MiB or 16 nested Forms.",
            ));
        }
        let content = Content::decode(&bytes)?;
        self.operations += content.operations.len();
        if self.operations > 500_000 {
            return Err(invalid(
                "Structure content inspection exceeds 500,000 drawing operations.",
            ));
        }
        let mut stack = Vec::new();
        let mut inside = false;
        let mut graphics_depth = 0_usize;
        let mut skip_until = None;
        for (index, operation) in content.operations.iter().enumerate() {
            if skip_until.is_some_and(|end| index <= end) {
                continue;
            }
            if scope.stream.is_none()
                && prefix.is_empty()
                && let Some(span) = self
                    .ocr
                    .get(&scope.page)
                    .and_then(|spans| spans.iter().find(|span| span.start == index))
                    .cloned()
            {
                if inside {
                    return Err(invalid(
                        "A canonical OCR stream cannot be nested inside another text object.",
                    ));
                }
                self.meaningful(&context, Some(&span.text), true)?;
                skip_until = Some(span.end);
                continue;
            }
            match operation.operator.as_str() {
                "q" => graphics_depth += 1,
                "Q" => {
                    graphics_depth = graphics_depth
                        .checked_sub(1)
                        .ok_or_else(|| invalid("Unbalanced graphics state in tagged content."))?
                }
                "BT" => {
                    if inside {
                        return Err(invalid("Nested text objects in tagged content."));
                    }
                    inside = true;
                }
                "ET" => {
                    if !inside {
                        return Err(invalid("Unbalanced text object in tagged content."));
                    }
                    inside = false;
                }
                "BMC" | "BDC" => {
                    if stack.len() >= 64 {
                        return Err(invalid("Marked-content nesting exceeds 64 levels."));
                    }
                    let name = operation
                        .operands
                        .first()
                        .ok_or_else(|| invalid("A marked-content tag has no name."))?
                        .as_name()?;
                    if name == b"OC" {
                        return Err(invalid(
                            "Optional-content visibility cannot be represented safely by this structured reader.",
                        ));
                    }
                    let properties = if operation.operator == "BDC" {
                        if operation.operands.len() != 2 {
                            return Err(invalid("A marked-content property list is malformed."));
                        }
                        let properties = &operation.operands[1];
                        let properties = if let Ok(name) = properties.as_name() {
                            resource(document, resources, b"Properties", name)?
                        } else {
                            properties
                        };
                        Some(resolve(document, properties)?.as_dict()?)
                    } else {
                        if operation.operands.len() != 1 {
                            return Err(invalid("A marked-content tag is malformed."));
                        }
                        None
                    };
                    stack.push(context.clone());
                    if name == b"Artifact" {
                        context.artifact = true;
                        if properties.is_some_and(|value| value.has(b"MCID")) {
                            return Err(invalid("Artifact content cannot also own an MCID."));
                        }
                    } else if let Some(properties) = properties {
                        if let Ok(value) = properties.get(b"MCID") {
                            if context.binding.is_some() || context.artifact {
                                return Err(invalid(
                                    "Nested or artifact-contained MCIDs have ambiguous semantic ownership.",
                                ));
                            }
                            let binding = Binding {
                                scope: scope.clone(),
                                mcid: integer(document, value)?,
                            };
                            let actual = properties
                                .get(b"ActualText")
                                .ok()
                                .map(|value| text(document, value, super::MAX_TEXT_BYTES))
                                .transpose()?;
                            if self
                                .marks
                                .insert(
                                    binding.clone(),
                                    Mark {
                                        actual,
                                        ..Mark::default()
                                    },
                                )
                                .is_some()
                            {
                                return Err(invalid(
                                    "An MCID is repeated within the same page/Form content scope.",
                                ));
                            }
                            if self.marks.len() > MAX_ITEMS {
                                return Err(invalid(
                                    "The supported structure exceeds 2,000 marked-content items.",
                                ));
                            }
                            context.binding = Some(binding);
                        } else if properties.has(b"ActualText") {
                            return Err(invalid(
                                "Nested ActualText without its own unambiguous MCID is not supported by this structured reader.",
                            ));
                        }
                    }
                }
                "EMC" => {
                    context = stack
                        .pop()
                        .ok_or_else(|| invalid("Unbalanced marked-content end."))?
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    if !inside {
                        return Err(invalid("Text is drawn outside a valid text object."));
                    }
                    let mut path = prefix.to_vec();
                    path.push(index);
                    fn has_text(value: &Object) -> bool {
                        match value {
                            Object::String(value, _) => !value.is_empty(),
                            Object::Array(values) => values.iter().any(has_text),
                            _ => false,
                        }
                    }
                    if !operation.operands.iter().any(has_text) {
                        continue;
                    }
                    let value = runs.get(&path).ok_or_else(|| invalid("A text operation could not be accounted for by the source decoder."))?
                        .as_deref().ok_or_else(|| invalid("An existing tag binds text whose source font cannot be decoded reliably."))?;
                    if !value.is_empty() {
                        self.meaningful(&context, Some(value), true)?;
                    }
                }
                "Do" => {
                    let name = operation
                        .operands
                        .first()
                        .ok_or_else(|| invalid("An XObject invocation has no name."))?
                        .as_name()?;
                    let object = resource(document, resources, b"XObject", name)?;
                    let stream = resolve(document, object)?.as_stream()?;
                    if stream.dict.has(b"OC") {
                        return Err(invalid(
                            "Optional-content Form/image visibility is not supported by the structured reader.",
                        ));
                    }
                    match stream.dict.get(b"Subtype")?.as_name()? {
                        b"Image" => self.meaningful(&context, None, true)?,
                        b"Form" => {
                            let id = object.as_reference().map_err(|_| {
                                invalid("A Form has no stable indirect stream identifier.")
                            })?;
                            if !self.visiting.insert(id) {
                                return Err(invalid("Cyclic Form invocation in tagged content."));
                            }
                            let child_scope = Scope {
                                page: scope.page,
                                stream: Some(id),
                            };
                            if stream.dict.has(b"StructParents") {
                                if !self.forms.insert(id) {
                                    return Err(invalid(
                                        "A tagged Form stream is invoked more than once. Its /Pg ownership is ambiguous.",
                                    ));
                                }
                                self.register(document, &child_scope, &stream.dict)?;
                            }
                            let mut path = prefix.to_vec();
                            path.push(index);
                            self.scan(
                                document,
                                stream.get_plain_content_with_limit(MAX_EDIT_CONTENT_BYTES)?,
                                &form_resources(document, stream, resources)?,
                                (child_scope, &path),
                                context.clone(),
                                runs,
                            )?;
                            self.visiting.remove(&id);
                        }
                        _ => {
                            return Err(invalid(
                                "An unsupported XObject cannot be bound to a reliable structured reading view.",
                            ));
                        }
                    }
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "sh" => {
                    self.meaningful(&context, None, true)?
                }
                "BI" | "ID" | "EI" => {
                    return Err(invalid(
                        "Inline images are outside the supported structure inspection boundary.",
                    ));
                }
                "cm" | "gs" | "m" | "l" | "c" | "v" | "y" | "h" | "re" | "n" | "W" | "W*" | "w"
                | "J" | "j" | "M" | "d" | "ri" | "i" | "CS" | "cs" | "SC" | "SCN" | "sc"
                | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" | "Tc" | "Tw" | "Tz" | "TL"
                | "Tf" | "Tr" | "Ts" | "Td" | "TD" | "Tm" | "T*" | "MP" | "DP" => {}
                _ => {
                    return Err(invalid(format!(
                        "Unsupported content operator {} prevents complete structure inspection.",
                        operation.operator
                    )));
                }
            }
        }
        if inside || !stack.is_empty() || graphics_depth != 0 {
            return Err(invalid(
                "The tagged content has unbalanced text, marked content or graphics state.",
            ));
        }
        Ok(())
    }
}

pub(super) struct Reading {
    pub nodes: Vec<SemanticNode>,
    pub issues: Vec<String>,
    pub complete: bool,
}

fn standard_role(value: &str) -> bool {
    matches!(
        value,
        "Document"
            | "Part"
            | "Art"
            | "Sect"
            | "Div"
            | "P"
            | "H"
            | "H1"
            | "H2"
            | "H3"
            | "H4"
            | "H5"
            | "H6"
            | "Figure"
            | "Caption"
            | "Span"
            | "BlockQuote"
            | "Code"
            | "Note"
            | "L"
            | "LI"
            | "Lbl"
            | "LBody"
    )
}

fn role(name: &[u8], mapping: &BTreeMap<Vec<u8>, Vec<u8>>) -> Result<String> {
    let mut name = name;
    let mut seen = HashSet::new();
    loop {
        let value = std::str::from_utf8(name)
            .map_err(|_| invalid("A structure role has an invalid name."))?;
        if standard_role(value) {
            return Ok(value.to_owned());
        }
        if !seen.insert(name.to_vec()) || seen.len() > 32 {
            return Err(invalid("RoleMap contains a cycle or exceeds 32 mappings."));
        }
        name = mapping.get(name).ok_or_else(|| invalid(format!("Unsupported custom, table, formula or annotation role {value}. Its semantics are not guessed.")))?;
    }
}

fn number_tree(
    document: &Document,
    object: &Object,
    seen: &mut HashSet<ObjectId>,
    depth: usize,
    result: &mut BTreeMap<i64, Vec<Option<ObjectId>>>,
) -> Result<Option<(i64, i64)>> {
    if depth > 32 {
        return Err(invalid("ParentTree exceeds 32 levels."));
    }
    if let Object::Reference(id) = object
        && !seen.insert(*id)
    {
        return Err(invalid("ParentTree contains a cycle or shared node."));
    }
    let dictionary = resolve(document, object)?.as_dict()?;
    if dictionary.has(b"Nums") == dictionary.has(b"Kids") {
        return Err(invalid("ParentTree must contain either Nums or Kids."));
    }
    let mut range = None;
    let mut previous = None;
    if let Ok(values) = dictionary.get(b"Nums") {
        let values = resolve(document, values)?.as_array()?;
        if values.len() % 2 != 0 || values.len() > (MAX_ITEMS + MAX_PAGES) * 2 {
            return Err(invalid("ParentTree Nums is malformed or too large."));
        }
        for pair in values.chunks_exact(2) {
            let key = integer(document, &pair[0])?;
            if previous.is_some_and(|previous| key <= previous) {
                return Err(invalid("ParentTree keys are not strictly ordered."));
            }
            let values = resolve(document, &pair[1])?.as_array()?;
            if values.len() > 200_001 {
                return Err(invalid("A ParentTree reverse mapping is too large."));
            }
            let values = values.iter().map(|value| match value {
                Object::Null => Ok(None), Object::Reference(id) => Ok(Some(*id)),
                _ => Err(invalid("ParentTree reverse mappings must be indirect StructElem references or null.")),
            }).collect::<Result<Vec<_>>>()?;
            if result.insert(key, values).is_some() {
                return Err(invalid("ParentTree repeats a key."));
            }
            if range.is_none() {
                range = Some((key, key));
            } else if let Some((_, end)) = &mut range {
                *end = key;
            }
            previous = Some(key);
        }
    } else {
        for child in resolve(document, dictionary.get(b"Kids")?)?.as_array()? {
            let child_range = number_tree(document, child, seen, depth + 1, result)?
                .ok_or_else(|| invalid("A ParentTree child is empty."))?;
            if previous.is_some_and(|previous| child_range.0 <= previous) {
                return Err(invalid("ParentTree child ranges overlap or are unordered."));
            }
            if range.is_none() {
                range = Some(child_range);
            } else if let Some((_, end)) = &mut range {
                *end = child_range.1;
            }
            previous = Some(child_range.1);
        }
    }
    if let Ok(limits) = dictionary.get(b"Limits") {
        let limits = resolve(document, limits)?.as_array()?;
        if limits.len() != 2
            || range
                != Some((
                    integer(document, &limits[0])?,
                    integer(document, &limits[1])?,
                ))
        {
            return Err(invalid("ParentTree Limits do not match its actual keys."));
        }
    } else if depth > 0 {
        return Err(invalid(
            "A ParentTree child is missing its required Limits.",
        ));
    }
    if result.len() > MAX_ITEMS + MAX_PAGES {
        return Err(invalid("ParentTree exceeds its bounded key budget."));
    }
    Ok(range)
}

fn id_tree(
    document: &Document,
    object: &Object,
    seen: &mut HashSet<ObjectId>,
    depth: usize,
    result: &mut BTreeMap<Vec<u8>, ObjectId>,
) -> Result<()> {
    if depth > 32 || result.len() > MAX_ITEMS {
        return Err(invalid("IDTree exceeds its supported size or depth."));
    }
    if let Object::Reference(id) = object
        && !seen.insert(*id)
    {
        return Err(invalid("IDTree contains a cycle or shared node."));
    }
    let dictionary = resolve(document, object)?.as_dict()?;
    if dictionary.has(b"Names") == dictionary.has(b"Kids") {
        return Err(invalid("IDTree must contain either Names or Kids."));
    }
    if let Ok(values) = dictionary.get(b"Names") {
        let values = resolve(document, values)?.as_array()?;
        if values.len() % 2 != 0 {
            return Err(invalid("IDTree has an unmatched name."));
        }
        let mut previous: Option<Vec<u8>> = None;
        for pair in values.chunks_exact(2) {
            let name = resolve(document, &pair[0])?.as_str()?.to_vec();
            if name.is_empty() || previous.as_ref().is_some_and(|value| name <= *value) {
                return Err(invalid("IDTree names are empty, duplicate or unordered."));
            }
            let id = pair[1].as_reference()?;
            if result.insert(name.clone(), id).is_some() {
                return Err(invalid("IDTree repeats an element ID."));
            }
            previous = Some(name);
        }
    } else {
        for child in resolve(document, dictionary.get(b"Kids")?)?.as_array()? {
            id_tree(document, child, seen, depth + 1, result)?;
        }
    }
    Ok(())
}

struct Tree<'a> {
    document: &'a Document,
    content: &'a ContentMap,
    pages: HashMap<ObjectId, usize>,
    roles: BTreeMap<Vec<u8>, Vec<u8>>,
    seen: HashSet<ObjectId>,
    bindings: HashMap<Binding, ObjectId>,
    ids: BTreeMap<Vec<u8>, ObjectId>,
}

impl Tree<'_> {
    fn bind(
        &mut self,
        object: &Object,
        page: Option<ObjectId>,
        element: ObjectId,
    ) -> Result<ContentBinding> {
        let (page, stream, mcid) = if matches!(object, Object::Integer(_)) {
            (page, None, integer(self.document, object)?)
        } else {
            let dictionary = resolve(self.document, object)?.as_dict()?;
            if dictionary.get(b"Type")?.as_name()? != b"MCR" {
                return Err(invalid(
                    "OBJR, annotations and unknown content references are outside the supported reading boundary.",
                ));
            }
            let page = dictionary
                .get(b"Pg")
                .ok()
                .map(Object::as_reference)
                .transpose()?
                .or(page);
            let stream = dictionary
                .get(b"Stm")
                .ok()
                .map(Object::as_reference)
                .transpose()?;
            if dictionary.has(b"StmOwn") {
                return Err(invalid(
                    "External stream-owner MCR associations are not supported.",
                ));
            }
            (
                page,
                stream,
                integer(self.document, dictionary.get(b"MCID")?)?,
            )
        };
        let page = page
            .and_then(|id| self.pages.get(&id).copied())
            .ok_or_else(|| {
                invalid("A content association has no valid /Pg in the document page tree.")
            })?;
        let binding = Binding {
            scope: Scope { page, stream },
            mcid,
        };
        let mark = self.content.marks.get(&binding).ok_or_else(|| {
            invalid(
                "A StructElem points to a missing MCID, wrong /Pg or wrong page/Form /Stm scope.",
            )
        })?;
        if !mark.painted {
            return Err(invalid(
                "A StructElem binds an empty marked-content sequence.",
            ));
        }
        if self.bindings.insert(binding.clone(), element).is_some() {
            return Err(invalid(
                "The same MCID is owned by multiple structure elements.",
            ));
        }
        if !self.content.scopes.contains_key(&binding.scope) {
            return Err(invalid(
                "Marked content has no owning page/Form StructParents key.",
            ));
        }
        Ok(ContentBinding {
            page,
            stream: stream.map(key),
            mcid,
        })
    }

    fn children(
        &mut self,
        value: &Object,
        parent: ObjectId,
        page: Option<ObjectId>,
        depth: usize,
        nodes: &mut Vec<SemanticNode>,
        bindings: &mut Vec<ContentBinding>,
    ) -> Result<()> {
        if depth > 64 {
            return Err(invalid("The structure tree exceeds 64 levels."));
        }
        if let Object::Array(values) = value {
            if values.len() > MAX_ITEMS {
                return Err(invalid("A structure child list exceeds 2,000 items."));
            }
            for value in values {
                self.children(value, parent, page, depth + 1, nodes, bindings)?;
            }
            return Ok(());
        }
        if let Object::Reference(id) = value {
            let dictionary = resolve(self.document, value)?.as_dict()?;
            if dictionary
                .get(b"Type")
                .is_ok_and(|value| value.as_name().is_ok_and(|value| value == b"StructElem"))
            {
                if !self.seen.insert(*id) || self.seen.len() > MAX_ITEMS {
                    return Err(invalid(
                        "The structure tree contains a cycle, shared StructElem or more than 2,000 elements.",
                    ));
                }
                if dictionary.get(b"P")?.as_reference()? != parent {
                    return Err(invalid("A StructElem has an incorrect parent /P."));
                }
                let role = role(dictionary.get(b"S")?.as_name()?, &self.roles)?;
                let page = dictionary
                    .get(b"Pg")
                    .ok()
                    .map(Object::as_reference)
                    .transpose()?
                    .or(page);
                if page.is_some_and(|id| !self.pages.contains_key(&id)) {
                    return Err(invalid("A StructElem /Pg is not a document page."));
                }
                if let Ok(value) = dictionary.get(b"ID") {
                    let value = resolve(self.document, value)?.as_str()?.to_vec();
                    if value.is_empty()
                        || value.len() > 512
                        || self.ids.insert(value, *id).is_some()
                    {
                        return Err(invalid("StructElem IDs are empty, duplicate or too large."));
                    }
                }
                let language = dictionary
                    .get(b"Lang")
                    .ok()
                    .map(|value| text(self.document, value, 63))
                    .transpose()?;
                if let Some(value) = &language {
                    super::language(value)?;
                }
                let alt = dictionary
                    .get(b"Alt")
                    .ok()
                    .map(|value| text(self.document, value, 8192))
                    .transpose()?;
                if role == "Figure" && alt.as_ref().is_none_or(|value| value.trim().is_empty()) {
                    return Err(invalid(
                        "A Figure is missing a nonempty human alternative. An empty Alt is not a meaningful description or an Artifact assignment.",
                    ));
                }
                if [b"Ref".as_slice(), b"NS", b"Phoneme", b"PhoneticAlphabet"]
                    .iter()
                    .any(|name| dictionary.has(name))
                {
                    return Err(invalid(
                        "Structure namespaces, references or phonetic semantics are outside this bounded reader.",
                    ));
                }
                let mut children = Vec::new();
                let mut direct = Vec::new();
                if let Ok(value) = dictionary.get(b"K") {
                    self.children(value, *id, page, depth + 1, &mut children, &mut direct)?;
                }
                if !direct.is_empty() && !children.is_empty() {
                    return Err(invalid(
                        "Interleaved direct content and nested structure children are not supported without losing their exact logical order.",
                    ));
                }
                if direct.is_empty() && children.is_empty() {
                    return Err(invalid("A supported structure element is empty."));
                }
                let mut parts = Vec::new();
                for binding in &direct {
                    let stream = binding
                        .stream
                        .as_deref()
                        .map(crate::pdf_objects::parse_key)
                        .transpose()?;
                    let mark = &self.content.marks[&Binding {
                        scope: Scope {
                            page: binding.page,
                            stream,
                        },
                        mcid: binding.mcid,
                    }];
                    parts.push(mark.actual.as_deref().unwrap_or(&mark.text).to_owned());
                }
                nodes.push(SemanticNode {
                    id: key(*id),
                    role,
                    language,
                    alt,
                    text: (!parts.is_empty()).then(|| parts.join(" ")),
                    bindings: direct,
                    children,
                });
                return Ok(());
            }
        }
        if parent
            == self
                .document
                .catalog()?
                .get(b"StructTreeRoot")?
                .as_reference()?
        {
            return Err(invalid(
                "StructTreeRoot children must be indirect StructElem nodes.",
            ));
        }
        bindings.push(self.bind(value, page, parent)?);
        Ok(())
    }
}

pub(super) fn inspect(document: &Document) -> Result<Reading> {
    if document.get_pages().len() > MAX_PAGES {
        return Err(invalid("Structured reading is limited to 100 pages."));
    }
    let catalog = document.catalog()?;
    let root_object = catalog.get(b"StructTreeRoot")?;
    let root_id = root_object
        .as_reference()
        .map_err(|_| invalid("StructTreeRoot must have a stable indirect identifier."))?;
    let root = resolve(document, root_object)?.as_dict()?;
    if root.get(b"Type")?.as_name()? != b"StructTreeRoot" {
        return Err(invalid("The catalog structure root has an incorrect type."));
    }
    let mut content = ContentMap::default();
    let mut pages = HashMap::new();
    let mut issues = Vec::new();
    let mut ocr = crate::ocr::canonical::Reader::default();
    for (page, id) in document.get_pages().values().enumerate() {
        if pages.insert(*id, page).is_some() {
            return Err(invalid("The page tree repeats a page object."));
        }
        let scope = Scope { page, stream: None };
        let dictionary = document.get_dictionary(*id)?;
        content.register(document, &scope, dictionary)?;
        let canonical = ocr.page(document, page)?;
        let analysis = crate::ocr::canonical::analysis_document(document, page, &canonical)?;
        content.ocr.insert(page, canonical);
        let runs = crate::text_edit::page_runs(&analysis, page)?
            .into_iter()
            .map(|run| (run.path, run.text))
            .collect();
        content.scan(
            document,
            document.get_page_content_with_limit(*id, MAX_EDIT_CONTENT_BYTES)?,
            &page_resources(document, *id)?,
            (scope, &[]),
            Context {
                binding: None,
                artifact: false,
            },
            &runs,
        )?;
        if dictionary.get(b"Annots").ok().is_some_and(|value| {
            resolve(document, value)
                .and_then(|value| Ok(!value.as_array()?.is_empty()))
                .unwrap_or(true)
        }) {
            issues.push(format!("Page {} has annotations or links. Their interaction/OBJR semantics are not represented in this reading view.", page + 1));
        }
    }
    if catalog.has(b"AcroForm") {
        issues.push(
            "Form fields are outside this reading view. Their semantics have not been checked."
                .into(),
        );
    }
    if catalog.has(b"OCProperties") {
        return Err(invalid(
            "Optional-content documents cannot be represented without guessing visibility.",
        ));
    }
    let mut roles = BTreeMap::new();
    if let Ok(value) = root.get(b"RoleMap") {
        for (name, value) in resolve(document, value)?.as_dict()? {
            if roles.len() >= 128 {
                return Err(invalid("RoleMap exceeds 128 entries."));
            }
            if standard_role(std::str::from_utf8(name).unwrap_or("")) {
                return Err(invalid(
                    "RoleMap attempts to redefine a standard structure role.",
                ));
            }
            roles.insert(name.clone(), resolve(document, value)?.as_name()?.to_vec());
        }
        for name in roles.keys() {
            role(name, &roles)?;
        }
    }
    let mut parents = BTreeMap::new();
    number_tree(
        document,
        root.get(b"ParentTree")?,
        &mut HashSet::new(),
        0,
        &mut parents,
    )?;
    if let Ok(value) = root.get(b"ParentTreeNextKey") {
        if parents
            .keys()
            .next_back()
            .is_some_and(|key| integer(document, value).is_ok_and(|value| value <= *key))
        {
            return Err(invalid(
                "ParentTreeNextKey does not exceed existing parent keys.",
            ));
        }
        integer(document, value)?;
    }
    let mut tree = Tree {
        document,
        content: &content,
        pages,
        roles,
        seen: HashSet::new(),
        bindings: HashMap::new(),
        ids: BTreeMap::new(),
    };
    let mut nodes = Vec::new();
    let mut direct = Vec::new();
    tree.children(root.get(b"K")?, root_id, None, 0, &mut nodes, &mut direct)?;
    if nodes.is_empty() || !direct.is_empty() {
        return Err(invalid(
            "The structure root has no supported element children.",
        ));
    }
    if tree.bindings.len() != content.marks.len() {
        return Err(invalid("Some MCIDs have no unique StructElem association."));
    }
    for (binding, element) in &tree.bindings {
        let key = content.scopes[&binding.scope];
        if parents
            .get(&key)
            .and_then(|values| values.get(binding.mcid as usize))
            .copied()
            .flatten()
            != Some(*element)
        {
            return Err(invalid(
                "ParentTree reverse mapping disagrees with the MCID's actual StructElem ownership.",
            ));
        }
    }
    for (key, values) in &parents {
        let scope = content.owners.get(key).ok_or_else(|| {
            invalid("ParentTree key has no actual page/Form StructParents owner.")
        })?;
        for (mcid, element) in values.iter().enumerate() {
            if let Some(element) = element
                && tree.bindings.get(&Binding {
                    scope: scope.clone(),
                    mcid: mcid as i64,
                }) != Some(element)
            {
                return Err(invalid(
                    "ParentTree contains an extra, stale or wrong-scope reverse association.",
                ));
            }
        }
    }
    for key in content.owners.keys() {
        if !parents.contains_key(key) {
            return Err(invalid(
                "A page/Form StructParents key is missing from ParentTree.",
            ));
        }
    }
    if let Ok(value) = root.get(b"IDTree") {
        let mut ids = BTreeMap::new();
        id_tree(document, value, &mut HashSet::new(), 0, &mut ids)?;
        if ids != tree.ids {
            return Err(invalid(
                "IDTree does not match the actual unique StructElem IDs.",
            ));
        }
    } else if !tree.ids.is_empty() {
        return Err(invalid("StructElem IDs have no matching IDTree."));
    }
    if content.unbound > 0 {
        issues.push(format!("{} meaningful drawing/text operations have no logical tag or explicit Artifact association. This is a partial reading view, not a complete document.", content.unbound));
    }
    match catalog
        .get(b"Lang")
        .ok()
        .map(|value| text(document, value, 63))
        .transpose()?
    {
        Some(value) => {
            if let Err(error) = super::language(&value) {
                issues.push(error.to_string());
            }
        }
        None => issues.push("Document language is missing.".into()),
    }
    if crate::metadata::inspect(document)?
        .fields
        .title
        .trim()
        .is_empty()
    {
        issues.push("Document title is missing.".into());
    }
    Ok(Reading {
        nodes,
        complete: issues.is_empty(),
        issues,
    })
}
