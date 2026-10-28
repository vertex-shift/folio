use std::{collections::BTreeMap, sync::Arc};

use lopdf::{Dictionary, Document, Object};

use crate::{
    document::{Result, invalid},
    pdf_objects::{resolve, stream_bytes},
    text_edit::{Codec, strict_codec},
    writing::{FONT, embedded_text_glyph},
};

const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
const MAX_WIDTHS: usize = 200_000;

pub(super) struct Font {
    pub object: Object,
    pub label: String,
    pub codec: Codec,
    pub original_reason: Option<String>,
    pub program_bytes: usize,
    program: Option<Program>,
    widths: Widths,
    bbox: [f64; 4],
    pub ascent: f64,
    pub descent: f64,
}

enum GlyphMap {
    Unicode,
    Identity,
    Cids(Vec<u16>),
}

struct Program {
    bytes: Vec<u8>,
    mapping: GlyphMap,
}

enum Widths {
    Simple {
        first: usize,
        values: Vec<f64>,
        missing: f64,
    },
    Cid {
        values: BTreeMap<u16, f64>,
        default: f64,
    },
    Standard(Vec<u16>),
}

#[derive(Clone, Debug)]
pub(super) struct Metric {
    pub advance: f64,
    pub ink: Option<[f64; 4]>,
}

#[derive(Clone)]
pub(super) struct Glyph {
    pub character: char,
    pub start: usize,
    pub end: usize,
    pub metric: Metric,
}

pub(super) struct Measured {
    pub glyphs: Vec<Glyph>,
    pub ascent: f64,
    pub descent: f64,
}

fn number(document: &Document, value: &Object) -> Result<f64> {
    let value = f64::from(resolve(document, value)?.as_float()?);
    if !value.is_finite() || value.abs() > 1_000_000.0 {
        return Err(invalid("The source font has invalid metrics."));
    }
    Ok(value)
}

fn array_numbers(document: &Document, value: &Object) -> Result<Vec<f64>> {
    let array = resolve(document, value)?.as_array()?;
    if array.len() > MAX_WIDTHS {
        return Err(invalid("The source font has too many widths."));
    }
    array.iter().map(|value| number(document, value)).collect()
}

impl Font {
    pub fn load(document: &Document, object: &Object) -> Result<Arc<Self>> {
        let dictionary = resolve(document, object)?.as_dict()?;
        let codec = strict_codec(document, dictionary).map_err(invalid)?;
        let label: String = dictionary
            .get(b"BaseFont")
            .and_then(Object::as_name)
            .map(|name| String::from_utf8_lossy(name).chars().take(160).collect())
            .unwrap_or_else(|_| "Source font".into());
        let composite = dictionary.get(b"Subtype")?.as_name()? == b"Type0";
        let metrics = if composite {
            let descendants = resolve(document, dictionary.get(b"DescendantFonts")?)?.as_array()?;
            if descendants.len() != 1 {
                return Err(invalid(
                    "A paragraph font needs one horizontal descendant font.",
                ));
            }
            resolve(document, &descendants[0])?.as_dict()?
        } else {
            dictionary
        };
        let descriptor = metrics
            .get(b"FontDescriptor")
            .ok()
            .map(|value| -> Result<_> { Ok(resolve(document, value)?.as_dict()?) })
            .transpose()?;
        let standard = standard_metrics(&label);
        let bbox = if let Some(descriptor) = descriptor {
            let values = array_numbers(document, descriptor.get(b"FontBBox")?)?;
            if values.len() != 4 || values[2] <= values[0] || values[3] <= values[1] {
                return Err(invalid("The source font has an invalid ink rectangle."));
            }
            [values[0], values[1], values[2], values[3]]
        } else if let Some((_, bbox)) = &standard {
            *bbox
        } else {
            return Err(invalid(
                "The source font has no reliable metrics or standard-font fallback.",
            ));
        };
        let ascent = descriptor
            .and_then(|value| value.get(b"Ascent").ok())
            .map(|value| number(document, value))
            .transpose()?
            .unwrap_or(bbox[3])
            .max(bbox[3]);
        let descent = descriptor
            .and_then(|value| value.get(b"Descent").ok())
            .map(|value| number(document, value))
            .transpose()?
            .unwrap_or(bbox[1])
            .min(bbox[1])
            .abs();
        let widths = if composite {
            cid_widths(document, metrics)?
        } else if let Ok(value) = metrics.get(b"Widths") {
            let first = resolve(document, metrics.get(b"FirstChar")?)?.as_i64()?;
            let last = resolve(document, metrics.get(b"LastChar")?)?.as_i64()?;
            let values = array_numbers(document, value)?;
            if !(0..=255).contains(&first)
                || last < first
                || last > 255
                || values.len() != (last - first + 1) as usize
            {
                return Err(invalid(
                    "The source font has inconsistent simple-font widths.",
                ));
            }
            let missing = descriptor
                .and_then(|value| value.get(b"MissingWidth").ok())
                .map(|value| number(document, value))
                .transpose()?
                .unwrap_or(0.0);
            Widths::Simple {
                first: first as usize,
                values,
                missing,
            }
        } else if let Some((widths, _)) = standard {
            Widths::Standard(widths)
        } else {
            return Err(invalid(
                "The source font does not declare reliable text advances.",
            ));
        };
        let mut original_reason = Some("The original font has no supported embedded TrueType ink metrics. Reflow requires an explicit conversion to Noto Sans; weight, tracking and kerning are not preserved.".into());
        let mut program = None;
        if let Some(descriptor) = descriptor
            && let Ok(value) = descriptor.get(b"FontFile2")
        {
            let bytes = stream_bytes(resolve(document, value)?.as_stream()?, MAX_FONT_BYTES)?;
            let face = ttf_parser::Face::parse(&bytes, 0)
                .map_err(|_| invalid("The embedded TrueType font cannot be measured reliably."))?;
            let mapping = if composite {
                if metrics.get(b"Subtype")?.as_name()? != b"CIDFontType2" {
                    return Err(invalid(
                        "Only CIDFontType2 TrueType paragraph fonts are supported.",
                    ));
                }
                match metrics.get(b"CIDToGIDMap") {
                    Ok(value) => match resolve(document, value)? {
                        Object::Name(name) if name == b"Identity" => GlyphMap::Identity,
                        Object::Stream(stream) => {
                            let bytes = stream_bytes(stream, 131_072)?;
                            if bytes.len() % 2 != 0 {
                                return Err(invalid("The font's CID-to-glyph map is incomplete."));
                            }
                            GlyphMap::Cids(
                                bytes
                                    .chunks_exact(2)
                                    .map(|value| u16::from_be_bytes([value[0], value[1]]))
                                    .collect(),
                            )
                        }
                        _ => return Err(invalid("The font's CID-to-glyph map is unsupported.")),
                    },
                    Err(lopdf::Error::DictKey(_)) => GlyphMap::Identity,
                    Err(error) => return Err(error.into()),
                }
            } else {
                let flags = descriptor
                    .get(b"Flags")
                    .ok()
                    .map(Object::as_i64)
                    .transpose()?
                    .unwrap_or(0);
                if flags & 4 != 0 || flags & 32 == 0 {
                    return Err(invalid(
                        "Symbolic simple TrueType fonts cannot be mapped to paragraph glyphs reliably.",
                    ));
                }
                GlyphMap::Unicode
            };
            if face.is_variable() {
                return Err(invalid(
                    "Variable source fonts cannot be measured without their variation instance.",
                ));
            }
            original_reason = None;
            program = Some(Program { bytes, mapping });
        }
        let program_bytes = program.as_ref().map_or(0, |program| program.bytes.len());
        Ok(Arc::new(Self {
            object: object.clone(),
            label,
            codec,
            original_reason,
            program_bytes,
            program,
            widths,
            bbox,
            ascent,
            descent,
        }))
    }

    pub fn metric(&self, bytes: &[u8], character: char) -> Result<Metric> {
        let code = match bytes {
            [value] => u16::from(*value),
            [a, b] => u16::from_be_bytes([*a, *b]),
            _ => {
                return Err(invalid(
                    "Paragraph source codes must map one glyph to one character.",
                ));
            }
        };
        let advance = match &self.widths {
            Widths::Simple {
                first,
                values,
                missing,
            } => usize::from(code)
                .checked_sub(*first)
                .and_then(|index| values.get(index))
                .copied()
                .unwrap_or(*missing),
            Widths::Cid { values, default } => values.get(&code).copied().unwrap_or(*default),
            Widths::Standard(widths) => {
                if !character.is_ascii() || !(32..=126).contains(&(character as u32)) {
                    return Err(invalid(
                        "This unembedded standard font only has reliable paragraph advances for printable ASCII. Use a PDF with explicit font widths for other source characters.",
                    ));
                }
                f64::from(widths[character as usize - 32])
            }
        };
        if !(0.0..=100_000.0).contains(&advance) {
            return Err(invalid(
                "The source font has a negative or unstable text advance.",
            ));
        }
        let ink = if let Some(program) = &self.program {
            let face = ttf_parser::Face::parse(&program.bytes, 0)
                .map_err(|_| invalid("The embedded font could not be measured."))?;
            let glyph = match &program.mapping {
                GlyphMap::Unicode => face.glyph_index(character),
                GlyphMap::Identity => Some(ttf_parser::GlyphId(code)),
                GlyphMap::Cids(map) => map.get(usize::from(code)).copied().map(ttf_parser::GlyphId),
            }
            .filter(|glyph| glyph.0 > 0 && glyph.0 < face.number_of_glyphs())
            .ok_or_else(|| {
                invalid("The source font does not resolve this encoded glyph reliably.")
            })?;
            let unit = 1000.0 / f64::from(face.units_per_em());
            if matches!(program.mapping, GlyphMap::Unicode) {
                let actual = f64::from(face.glyph_hor_advance(glyph).ok_or_else(|| {
                    invalid("An embedded source glyph has no horizontal advance.")
                })?) * unit;
                if (actual - advance).abs() > 1.0 {
                    return Err(invalid(
                        "The simple font's Unicode glyph and PDF widths disagree; its glyph mapping is ambiguous.",
                    ));
                }
            }
            face.glyph_bounding_box(glyph).map(|bounds| {
                [
                    f64::from(bounds.x_min) * unit,
                    f64::from(bounds.y_min) * unit,
                    f64::from(bounds.x_max) * unit,
                    f64::from(bounds.y_max) * unit,
                ]
            })
        } else {
            (!character.is_whitespace()).then_some(self.bbox)
        };
        Ok(Metric { advance, ink })
    }
}

fn cid_widths(document: &Document, font: &Dictionary) -> Result<Widths> {
    let default = font
        .get(b"DW")
        .ok()
        .map(|value| number(document, value))
        .transpose()?
        .unwrap_or(1000.0);
    let mut values = BTreeMap::new();
    if let Ok(widths) = font.get(b"W") {
        let widths = resolve(document, widths)?.as_array()?;
        let mut index = 0;
        while index < widths.len() {
            let start = resolve(document, &widths[index])?.as_i64()?;
            index += 1;
            let next = widths
                .get(index)
                .ok_or_else(|| invalid("The CID font widths are incomplete."))?;
            index += 1;
            if let Object::Array(array) = resolve(document, next)? {
                if start < 0 || start as usize + array.len() > 65_536 {
                    return Err(invalid(
                        "The CID font widths exceed their source-code range.",
                    ));
                }
                for (offset, value) in array.iter().enumerate() {
                    if values
                        .insert((start as usize + offset) as u16, number(document, value)?)
                        .is_some()
                    {
                        return Err(invalid(
                            "The CID font contains overlapping width declarations.",
                        ));
                    }
                }
            } else {
                let end = resolve(document, next)?.as_i64()?;
                let width = number(
                    document,
                    widths
                        .get(index)
                        .ok_or_else(|| invalid("The CID font widths are incomplete."))?,
                )?;
                index += 1;
                if start < 0 || end < start || end > 65_535 {
                    return Err(invalid("The CID font widths have an invalid range."));
                }
                for code in start..=end {
                    if values.insert(code as u16, width).is_some() {
                        return Err(invalid(
                            "The CID font contains overlapping width declarations.",
                        ));
                    }
                }
            }
            if values.len() > MAX_WIDTHS {
                return Err(invalid("The CID font has too many widths."));
            }
        }
    }
    Ok(Widths::Cid { values, default })
}

pub(super) fn measure(text: &str, size: f64, original: Option<(&Font, &[u8])>) -> Result<Measured> {
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|_| invalid("The bundled Noto Sans font is invalid."))?;
    let mut glyphs = Vec::new();
    let (ascent, descent) = if let Some((font, _)) = original {
        if let Some(reason) = &font.original_reason {
            return Err(invalid(reason));
        }
        (font.ascent * size / 1000.0, font.descent * size / 1000.0)
    } else {
        let scale = size / f64::from(face.units_per_em());
        (
            f64::from(face.ascender()) * scale,
            -f64::from(face.descender()) * scale,
        )
    };
    for (start, character) in text.char_indices() {
        let metric =
            if character == '\n' {
                Metric {
                    advance: 0.0,
                    ink: None,
                }
            } else if let Some((font, preferred)) = original {
                let encoded = font.codec.encode(&character.to_string(), preferred)?;
                let mut metric = font.metric(&encoded, character)?;
                metric.advance *= size / 1000.0;
                metric.ink = metric
                    .ink
                    .map(|bounds| bounds.map(|value| value * size / 1000.0));
                metric
            } else {
                let glyph = embedded_text_glyph(&face, character)?;
                let scale = size / f64::from(face.units_per_em());
                Metric {
                    advance: f64::from(
                        f32::from(face.glyph_hor_advance(glyph).ok_or_else(|| {
                            invalid("A Noto Sans glyph has no horizontal advance.")
                        })?) * (1000.0 / f32::from(face.units_per_em())),
                    ) * size
                        / 1000.0,
                    ink: face.glyph_bounding_box(glyph).map(|bounds| {
                        [
                            f64::from(bounds.x_min) * scale,
                            f64::from(bounds.y_min) * scale,
                            f64::from(bounds.x_max) * scale,
                            f64::from(bounds.y_max) * scale,
                        ]
                    }),
                }
            };
        glyphs.push(Glyph {
            character,
            start,
            end: start + character.len_utf8(),
            metric,
        });
    }
    Ok(Measured {
        glyphs,
        ascent,
        descent,
    })
}

fn standard_metrics(name: &str) -> Option<(Vec<u16>, [f64; 4])> {
    let (widths, bbox): (&[u16], [f64; 4]) = match name {
        "Helvetica" | "Helvetica-Oblique" => (&HELVETICA, [-170.0, -225.0, 1116.0, 931.0]),
        "Helvetica-Bold" | "Helvetica-BoldOblique" => {
            (&HELVETICA_BOLD, [-174.0, -228.0, 1114.0, 962.0])
        }
        "Times-Roman" => (&TIMES, [-168.0, -218.0, 1000.0, 898.0]),
        "Courier" | "Courier-Bold" | "Courier-Oblique" | "Courier-BoldOblique" => {
            (&[600; 95], [-57.0, -250.0, 869.0, 805.0])
        }
        _ => return None,
    };
    Some((widths.to_vec(), bbox))
}

const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];
const TIMES: [u16; 95] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667, 722, 611,
    556, 722, 722, 333, 389, 722, 611, 889, 722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722,
    722, 611, 333, 278, 333, 469, 500, 333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500,
    278, 778, 500, 500, 500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
];
