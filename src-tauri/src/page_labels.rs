use std::collections::{BTreeMap, HashSet};

use lopdf::{Document, Object, ObjectId, dictionary, text_string};
use serde::{Deserialize, Serialize};

use crate::document::{Result, invalid};

pub const MAX_EDIT_RANGES: usize = 256;
const MAX_PAGES: usize = 10_000;
const MAX_PREFIX_BYTES: usize = 512;
const MAX_LABEL_BYTES: usize = 1024;
const MAX_TOTAL_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Decimal,
    RomanUpper,
    RomanLower,
    LettersUpper,
    LettersLower,
    Prefix,
}

impl Style {
    fn name(self) -> Option<&'static str> {
        match self {
            Self::Decimal => Some("D"),
            Self::RomanUpper => Some("R"),
            Self::RomanLower => Some("r"),
            Self::LettersUpper => Some("A"),
            Self::LettersLower => Some("a"),
            Self::Prefix => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub page: usize,
    pub style: Style,
    pub prefix: String,
    pub start: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Info {
    pub labels: Option<Vec<String>>,
    pub ranges: Vec<Range>,
    pub issue: Option<String>,
}

#[derive(Clone)]
struct Numbering {
    style: Style,
    prefix: String,
    number: u32,
}

struct Parsed {
    ranges: Vec<Range>,
    numbering: Vec<Numbering>,
    labels: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Snapshot {
    present: bool,
    pages: BTreeMap<ObjectId, Numbering>,
}

impl Snapshot {
    pub fn capture(document: &Document) -> Result<Self> {
        let Some(parsed) = read(document)? else {
            return Ok(Self::default());
        };
        let pages: BTreeMap<_, _> = document
            .get_pages()
            .into_values()
            .zip(parsed.numbering)
            .collect();
        if pages.len() != parsed.labels.len() {
            return Err(invalid(
                "Page labels cannot be preserved for repeated page-object references.",
            ));
        }
        Ok(Self {
            present: true,
            pages,
        })
    }

    pub fn duplicate(&mut self, source: ObjectId, copy: ObjectId) {
        if let Some(label) = self.pages.get(&source).cloned() {
            self.pages.insert(copy, label);
        }
    }

    pub fn import(
        &mut self,
        donor: Self,
        original: &[ObjectId],
        renumbered: &[ObjectId],
    ) -> Result<()> {
        if original.len() != renumbered.len() {
            return Err(invalid(
                "Imported page labels do not match the imported pages.",
            ));
        }
        self.present |= donor.present;
        for (old, new) in original.iter().zip(renumbered) {
            if let Some(label) = donor.pages.get(old) {
                self.pages.insert(*new, label.clone());
            }
        }
        Ok(())
    }

    pub fn restore(self, document: &mut Document, order: &[ObjectId]) -> Result<()> {
        if !self.present {
            document.catalog_mut()?.remove(b"PageLabels");
            return Ok(());
        }
        let mut ranges: Vec<Range> = Vec::new();
        for (page, id) in order.iter().enumerate() {
            let label = self.pages.get(id).cloned().unwrap_or(Numbering {
                style: Style::Decimal,
                prefix: String::new(),
                number: u32::try_from(page + 1)
                    .map_err(|_| invalid("This PDF has too many pages for page labels."))?,
            });
            let continues = ranges.last().is_some_and(|previous| {
                previous.style == label.style
                    && previous.prefix == label.prefix
                    && (label.style == Style::Prefix
                        || previous.start.checked_add((page - previous.page) as u32)
                            == Some(label.number))
            });
            if !continues {
                ranges.push(Range {
                    page,
                    style: label.style,
                    prefix: label.prefix,
                    start: label.number,
                });
            }
        }
        resolve(&ranges, order.len())?;
        write(document, &ranges)
    }
}

pub fn inspect(document: &Document) -> Option<Info> {
    match read(document) {
        Ok(None) => None,
        Ok(Some(parsed)) => Some(Info {
            labels: Some(parsed.labels),
            ranges: parsed.ranges,
            issue: None,
        }),
        Err(error) => Some(Info {
            labels: None,
            ranges: Vec::new(),
            issue: Some(format!(
                "PDF page labels could not be read. Physical page positions are shown. {error}"
            )),
        }),
    }
}

pub(crate) fn update(document: &mut Document, ranges: &[Range]) -> Result<()> {
    if ranges.len() > MAX_EDIT_RANGES {
        return Err(invalid("Edit at most 256 page-label sections at once."));
    }
    if !ranges.is_empty() {
        resolve(ranges, document.get_pages().len())?;
    }
    write(document, ranges)
}

fn write(document: &mut Document, ranges: &[Range]) -> Result<()> {
    if ranges.is_empty() {
        document.catalog_mut()?.remove(b"PageLabels");
        return Ok(());
    }
    let mut nums = Vec::with_capacity(ranges.len() * 2);
    for range in ranges {
        let mut definition = dictionary! {};
        if let Some(style) = range.style.name() {
            definition.set("S", Object::Name(style.as_bytes().to_vec()));
            definition.set("St", i64::from(range.start));
        }
        if !range.prefix.is_empty() {
            definition.set("P", text_string(&range.prefix));
        }
        nums.push(Object::Integer(range.page as i64));
        nums.push(Object::Dictionary(definition));
    }
    let tree = document.add_object(dictionary! {
        "Nums" => nums,
        "Limits" => vec![Object::Integer(0), Object::Integer(ranges.last().unwrap().page as i64)],
    });
    document.catalog_mut()?.set("PageLabels", tree);
    Ok(())
}

fn read(document: &Document) -> Result<Option<Parsed>> {
    let tree = match document.catalog()?.get(b"PageLabels") {
        Ok(value) => value,
        Err(lopdf::Error::DictKey(_)) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut ranges = Vec::new();
    collect(document, tree, &mut ranges, &mut HashSet::new(), &mut 0, 0)?;
    let (numbering, labels) = resolve(&ranges, document.get_pages().len())?;
    Ok(Some(Parsed {
        ranges,
        numbering,
        labels,
    }))
}

fn collect(
    document: &Document,
    value: &Object,
    ranges: &mut Vec<Range>,
    visited: &mut HashSet<ObjectId>,
    nodes: &mut usize,
    depth: usize,
) -> Result<()> {
    *nodes += 1;
    if depth > 32 || *nodes > 2048 || ranges.len() > MAX_PAGES {
        return Err(invalid(
            "The PDF page-label tree is too large or deeply nested.",
        ));
    }
    let (id, object) = document.dereference(value)?;
    if let Some(id) = id
        && !visited.insert(id)
    {
        return Err(invalid(
            "The PDF page-label tree contains a cycle or shared node.",
        ));
    }
    let node = object.as_dict()?;
    let first = ranges.len();
    if node.has(b"Nums") == node.has(b"Kids") {
        return Err(invalid(
            "A PDF page-label node must contain Nums or Kids, not both.",
        ));
    }
    if let Ok(nums) = node.get(b"Nums") {
        let nums = document.dereference(nums)?.1.as_array()?;
        if nums.is_empty() || nums.len() % 2 != 0 || nums.len() / 2 > MAX_PAGES {
            return Err(invalid(
                "The PDF page-label number tree is malformed or excessive.",
            ));
        }
        for pair in nums.chunks_exact(2) {
            if ranges.len() >= MAX_PAGES {
                return Err(invalid("The PDF has more than 10,000 page-label sections."));
            }
            let page = usize::try_from(document.dereference(&pair[0])?.1.as_i64()?)
                .map_err(|_| invalid("A page-label section has a negative page position."))?;
            if ranges.last().is_some_and(|previous| previous.page >= page) {
                return Err(invalid(
                    "Page-label sections must have distinct ascending positions.",
                ));
            }
            let definition = document.dereference(&pair[1])?.1.as_dict()?;
            if definition
                .iter()
                .any(|(key, _)| ![b"Type".as_slice(), b"S", b"P", b"St"].contains(&key.as_slice()))
            {
                return Err(invalid(
                    "This page-label definition has unsupported properties.",
                ));
            }
            if let Ok(kind) = definition.get(b"Type")
                && document.dereference(kind)?.1.as_name()? != b"PageLabel"
            {
                return Err(invalid("A PDF page-label definition has an invalid type."));
            }
            let style = match definition.get(b"S") {
                Ok(value) => match document.dereference(value)?.1.as_name()? {
                    b"D" => Style::Decimal,
                    b"R" => Style::RomanUpper,
                    b"r" => Style::RomanLower,
                    b"A" => Style::LettersUpper,
                    b"a" => Style::LettersLower,
                    _ => return Err(invalid("This PDF uses an unsupported page-label style.")),
                },
                Err(lopdf::Error::DictKey(_)) => Style::Prefix,
                Err(error) => return Err(error.into()),
            };
            let prefix = match definition.get(b"P") {
                Ok(value) => crate::pdf_objects::text(document, value, MAX_PREFIX_BYTES)?,
                Err(lopdf::Error::DictKey(_)) => String::new(),
                Err(error) => return Err(error.into()),
            };
            let start = match definition.get(b"St") {
                Ok(value) => u32::try_from(document.dereference(value)?.1.as_i64()?)
                    .map_err(|_| invalid("A page-label starting number is out of range."))?,
                Err(lopdf::Error::DictKey(_)) => 1,
                Err(error) => return Err(error.into()),
            };
            ranges.push(Range {
                page,
                style,
                prefix,
                start,
            });
        }
    } else {
        let kids = document.dereference(node.get(b"Kids")?)?.1.as_array()?;
        if kids.is_empty() || kids.len() > 2048 {
            return Err(invalid(
                "The PDF page-label tree has invalid or excessive children.",
            ));
        }
        for child in kids {
            collect(document, child, ranges, visited, nodes, depth + 1)?;
        }
    }
    if let Ok(limits) = node.get(b"Limits") {
        let limits = document.dereference(limits)?.1.as_array()?;
        if limits.len() != 2
            || limits[0].as_i64()? != ranges[first].page as i64
            || limits[1].as_i64()? != ranges.last().unwrap().page as i64
        {
            return Err(invalid("The PDF page-label tree has inconsistent limits."));
        }
    }
    Ok(())
}

fn resolve(ranges: &[Range], page_count: usize) -> Result<(Vec<Numbering>, Vec<String>)> {
    if page_count == 0
        || page_count > MAX_PAGES
        || ranges.is_empty()
        || ranges.len() > MAX_PAGES
        || ranges[0].page != 0
    {
        return Err(invalid(
            "Page labels need a section starting at page 1 and at most 10,000 pages.",
        ));
    }
    for (index, range) in ranges.iter().enumerate() {
        if range.page >= page_count
            || range.start == 0
            || index > 0 && ranges[index - 1].page >= range.page
            || range.prefix.len() > MAX_PREFIX_BYTES
            || range.prefix.chars().any(char::is_control)
        {
            return Err(invalid(
                "Page-label sections have invalid positions, numbers or prefixes.",
            ));
        }
    }
    let mut numbering = Vec::with_capacity(page_count);
    let mut labels = Vec::with_capacity(page_count);
    let mut current = 0;
    let mut total = 0;
    for page in 0..page_count {
        if current + 1 < ranges.len() && ranges[current + 1].page == page {
            current += 1;
        }
        let range = &ranges[current];
        let number = if range.style == Style::Prefix {
            1
        } else {
            range
                .start
                .checked_add((page - range.page) as u32)
                .ok_or_else(|| invalid("A page-label numbering sequence overflows."))?
        };
        let suffix = match range.style {
            Style::Decimal => number.to_string(),
            Style::RomanUpper | Style::RomanLower => {
                roman(number, range.style == Style::RomanLower)?
            }
            Style::LettersUpper | Style::LettersLower => {
                let count = (number - 1) / 26 + 1;
                if count as usize > MAX_LABEL_BYTES {
                    return Err(invalid(
                        "A repeated-letter page label exceeds its length limit.",
                    ));
                }
                let first = if range.style == Style::LettersLower {
                    b'a'
                } else {
                    b'A'
                };
                char::from(first + ((number - 1) % 26) as u8)
                    .to_string()
                    .repeat(count as usize)
            }
            Style::Prefix => String::new(),
        };
        let label = format!("{}{}", range.prefix, suffix);
        total += label.len();
        if label.len() > MAX_LABEL_BYTES || total > MAX_TOTAL_BYTES {
            return Err(invalid(
                "PDF page labels exceed the 1 KiB label or 1 MiB total text limit.",
            ));
        }
        numbering.push(Numbering {
            style: range.style,
            prefix: range.prefix.clone(),
            number,
        });
        labels.push(label);
    }
    Ok((numbering, labels))
}

fn roman(mut number: u32, lower: bool) -> Result<String> {
    if number / 1000 > MAX_LABEL_BYTES as u32 {
        return Err(invalid("A Roman page label exceeds its length limit."));
    }
    let mut result = String::new();
    for (value, text) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while number >= value {
            result.push_str(text);
            number -= value;
        }
    }
    Ok(if lower {
        result.to_ascii_lowercase()
    } else {
        result
    })
}
