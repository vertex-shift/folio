use std::collections::BTreeMap;

use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use ttf_parser::GlyphId;

use super::{
    Direction, Word,
    catalog::{FontId, Language, permits},
};
use crate::document::{Result, invalid};

pub fn word_font(word: &Word, languages: &[&Language]) -> Result<FontId> {
    for character in word.text.chars() {
        if character.is_control() || character.is_whitespace() || !permits(character, languages) {
            return Err(invalid(format!(
                "OCR contains unsupported or unselected character U+{:04X}. No characters were dropped and no text was added.",
                character as u32
            )));
        }
        if matches!(character as u32, 0x202a..=0x202e | 0x2066..=0x2069) {
            return Err(invalid(
                "OCR bidi override/isolate controls are not supported. No text was added.",
            ));
        }
    }
    let fonts = languages
        .iter()
        .map(|item| item.font)
        .chain([FontId::Latin]);
    for id in fonts {
        let face = ttf_parser::Face::parse(id.bytes(), 0)
            .map_err(|error| invalid(format!("The OCR font is invalid: {error}")))?;
        if word.text.chars().all(|character| {
            matches!(character, '\u{200c}' | '\u{200d}')
                || face
                    .glyph_index(character)
                    .is_some_and(|glyph| glyph.0 != 0)
        }) && word
            .text
            .chars()
            .any(|character| usable_glyph(&face, character).is_some())
        {
            if word.direction == Direction::Rtl
                && !languages
                    .iter()
                    .any(|language| language.scripts.iter().any(|script| script == "Arabic"))
            {
                return Err(invalid(
                    "An RTL OCR result requires the selected Arabic model. No English substitution was made.",
                ));
            }
            return Ok(id);
        }
    }
    Err(invalid(
        "A recognized word cannot be represented by a single selected OCR font with real glyphs. No text was added; choose the matching bundled language.",
    ))
}

fn usable_glyph(face: &ttf_parser::Face<'_>, character: char) -> Option<GlyphId> {
    face.glyph_index(character).filter(|glyph| {
        glyph.0 != 0
            && face
                .glyph_hor_advance(*glyph)
                .is_some_and(|width| width > 0)
            && face.glyph_bounding_box(*glyph).is_some()
    })
}

pub fn advance(face: &ttf_parser::Face<'_>, text: &str) -> f64 {
    text.chars()
        .filter_map(|character| {
            face.glyph_index(character)
                .and_then(|glyph| face.glyph_hor_advance(glyph))
        })
        .map(|value| f64::from(value) / f64::from(face.units_per_em()))
        .sum()
}

pub struct Embedded {
    pub object: ObjectId,
    pub encoded: BTreeMap<String, u16>,
}

pub fn embed(
    document: &mut Document,
    id: FontId,
    direction: Direction,
    words: &[&Word],
) -> Result<Embedded> {
    let mut unique = BTreeMap::new();
    for word in words {
        unique.entry(word.text.clone()).or_insert(false);
        if !word.separator.is_empty() {
            unique.entry(word.separator.clone()).or_insert(true);
        }
    }
    embed_entries(document, id, direction, unique)
}

pub(super) fn embed_entries(
    document: &mut Document,
    id: FontId,
    direction: Direction,
    unique: BTreeMap<String, bool>,
) -> Result<Embedded> {
    let data = id.bytes();
    let face = ttf_parser::Face::parse(data, 0)
        .map_err(|error| invalid(format!("The OCR font is invalid: {error}")))?;
    let unit = 1000.0 / f64::from(face.units_per_em());
    let name = format!("FolioOCRLogicalV1_{}_{}", id.key(), direction.key());
    if unique.len() > 60_000 {
        return Err(invalid(
            "OCR exceeds the 60,000 unique-word font encoding limit. No text was added.",
        ));
    }
    let mut font_file = Stream::new(dictionary! { "Length1"=>data.len() as i64 }, data.to_vec());
    font_file.compress()?;
    let file = document.add_object(font_file);
    let bbox = face.global_bounding_box();
    let descriptor = document.add_object(dictionary! {
        "Type"=>"FontDescriptor","FontName"=>name.clone(),"Flags"=>32,
        "FontBBox"=>[bbox.x_min,bbox.y_min,bbox.x_max,bbox.y_max].iter().map(|value| Object::Real((f64::from(*value)*unit) as f32)).collect::<Vec<_>>(),
        "ItalicAngle"=>0,"Ascent"=>f64::from(face.ascender())*unit,
        "Descent"=>f64::from(face.descender())*unit,"CapHeight"=>f64::from(face.capital_height().unwrap_or(face.ascender()))*unit,
        "StemV"=>80,"FontFile2"=>file,
    });
    let mut encoded = BTreeMap::new();
    let mut glyph_map = vec![0_u8; (unique.len() + 1) * 2];
    let mut widths = Vec::with_capacity(unique.len() * 2);
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /FolioOCRLogical def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries = unique.iter().enumerate().collect::<Vec<_>>();
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (index, (text, separator)) in chunk {
            let cid = (index + 1) as u16;
            let glyph = if **separator {
                face.glyph_index(' ').filter(|glyph| glyph.0 != 0)
            } else {
                text.chars()
                    .find_map(|character| usable_glyph(&face, character))
            }
            .ok_or_else(|| invalid("OCR never encodes an empty or .notdef placeholder glyph."))?;
            encoded.insert((*text).clone(), cid);
            glyph_map[usize::from(cid) * 2..usize::from(cid) * 2 + 2]
                .copy_from_slice(&glyph.0.to_be_bytes());
            widths.push(Object::Integer(i64::from(cid)));
            widths.push(Object::Array(vec![Object::Real(if **separator {
                0.0
            } else {
                (advance(&face, text) * 1000.0) as f32
            })]));
            let unicode: String = text
                .encode_utf16()
                .map(|code| format!("{code:04X}"))
                .collect();
            cmap.push_str(&format!("<{cid:04X}> <{unicode}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let unicode = document.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
    let map = document.add_object(Stream::new(dictionary! {}, glyph_map));
    let descendant = document.add_object(dictionary! {
        "Type"=>"Font","Subtype"=>"CIDFontType2","BaseFont"=>name.clone(),
        "CIDSystemInfo"=>dictionary!{"Registry"=>Object::string_literal("Adobe"),"Ordering"=>Object::string_literal("Identity"),"Supplement"=>0},
        "FontDescriptor"=>descriptor,"CIDToGIDMap"=>map,"DW"=>1000,"W"=>widths,
    });
    let object = document.add_object(dictionary! {
        "Type"=>"Font","Subtype"=>"Type0","BaseFont"=>name,
        "Encoding"=>"Identity-H","DescendantFonts"=>vec![Object::Reference(descendant)],"ToUnicode"=>unicode,
    });
    Ok(Embedded { object, encoded })
}
