use std::collections::BTreeMap;

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary, text_string,
};
use unicode_normalization::UnicodeNormalization;

use crate::document::{
    Result, invalid, numbers, page_bounds, page_rotation, validate_point, validate_text,
};

pub(crate) const FONT: &[u8] = include_bytes!("../resources/fonts/NotoSans-Regular.ttf");

fn font_glyphs(face: &ttf_parser::Face<'_>, text: &str) -> Result<BTreeMap<char, (u16, f32)>> {
    let unit = 1000.0 / f32::from(face.units_per_em());
    let mut glyphs = BTreeMap::new();
    for character in text.chars().filter(|character| *character != '\n') {
        let glyph = embedded_text_glyph(face, character)?;
        glyphs.entry(character).or_insert((
            glyph.0,
            f32::from(face.glyph_hor_advance(glyph).unwrap_or(face.units_per_em())) * unit,
        ));
    }
    Ok(glyphs)
}

pub(crate) fn line_metrics(text: &str, size: f32) -> Result<(f32, f32, f32)> {
    if text.contains('\n') {
        return Err(invalid("Text stamps require a single line."));
    }
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|error| invalid(format!("The bundled text font is invalid: {error}")))?;
    let glyphs = font_glyphs(&face, text)?;
    let scale = size / f32::from(face.units_per_em());
    Ok((
        text.chars()
            .map(|character| glyphs[&character].1 * size / 1000.0)
            .sum(),
        f32::from(face.ascender()) * scale,
        f32::from(face.descender()).abs() * scale,
    ))
}

pub(crate) fn validate_color(color: [f32; 3]) -> Result<()> {
    if color
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(invalid("Choose a valid RGB writing color."));
    }
    Ok(())
}

pub(crate) fn text_annotation(
    document: &mut Document,
    page: ObjectId,
    rect: [f32; 4],
    text: &str,
    size: f32,
    color: [f32; 3],
    revision: u64,
) -> Result<Dictionary> {
    let text: String = text.nfc().collect();
    validate_text(&text, false)?;
    validate_color(color)?;
    let bounds = page_bounds(document, page)?;
    validate_point([rect[0], rect[1]], bounds)?;
    validate_point([rect[2], rect[3]], bounds)?;
    if !size.is_finite()
        || !(6.0..=96.0).contains(&size)
        || rect[2] - rect[0] < 10.0
        || rect[3] - rect[1] < 10.0
    {
        return Err(invalid(
            "Choose a text size from 6 to 96 points and a text box inside the page.",
        ));
    }
    let rotation = page_rotation(document, page)?;
    let appearance = text_appearance(document, rect, &text, size, color, rotation)?;
    Ok(dictionary! {
        "Type" => "Annot", "Subtype" => "FreeText", "P" => page,
        "Rect" => numbers(&rect), "Contents" => text_string(&text),
        "T" => text_string("Folio"), "NM" => text_string(&format!("folio-{revision}-{}", document.max_id + 1)),
        "F" => 4, "C" => numbers(&color), "FolioFontSize" => size, "FolioRotation" => rotation,
        "DA" => Object::string_literal(format!("/FolioFont {size} Tf {} {} {} rg", color[0], color[1], color[2])),
        "Border" => vec![0.into(), 0.into(), 0.into()],
        "AP" => dictionary! { "N" => appearance },
    })
}

pub(crate) fn update_text(document: &mut Document, id: ObjectId, text: &str) -> Result<()> {
    let text: String = text.nfc().collect();
    validate_text(&text, false)?;
    let annotation = document.get_dictionary(id)?;
    let rectangle = annotation.get(b"Rect")?.as_array()?;
    let color = annotation.get(b"C")?.as_array()?;
    if rectangle.len() != 4 || color.len() != 3 {
        return Err(invalid(
            "The text annotation has invalid geometry or color.",
        ));
    }
    let rect = [
        rectangle[0].as_float()?,
        rectangle[1].as_float()?,
        rectangle[2].as_float()?,
        rectangle[3].as_float()?,
    ];
    let color = [
        color[0].as_float()?,
        color[1].as_float()?,
        color[2].as_float()?,
    ];
    let size = annotation.get(b"FolioFontSize")?.as_float()?;
    let rotation = annotation.get(b"FolioRotation")?.as_i64()? as i32;
    let appearance = text_appearance(document, rect, &text, size, color, rotation)?;
    let annotation = document.get_dictionary_mut(id)?;
    annotation.set("Contents", text_string(&text));
    annotation.set("AP", dictionary! { "N" => appearance });
    document.prune_objects();
    Ok(())
}

fn text_appearance(
    document: &mut Document,
    rect: [f32; 4],
    text: &str,
    size: f32,
    color: [f32; 3],
    rotation: i32,
) -> Result<ObjectId> {
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|error| invalid(format!("The bundled text font is invalid: {error}")))?;
    let unit = 1000.0 / f32::from(face.units_per_em());
    let glyphs = font_glyphs(&face, text)?;
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    let (local_width, local_height, matrix) = match rotation.rem_euclid(360) {
        0 => (width, height, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        90 => (height, width, [0.0, 1.0, -1.0, 0.0, width, 0.0]),
        180 => (width, height, [-1.0, 0.0, 0.0, -1.0, width, height]),
        270 => (height, width, [0.0, -1.0, 1.0, 0.0, 0.0, height]),
        _ => return Err(invalid("The text annotation has an invalid rotation.")),
    };
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0.0;
    let usable = local_width - 8.0;
    for character in text.chars() {
        if character == '\n' {
            lines.push(std::mem::take(&mut current));
            used = 0.0;
            continue;
        }
        let advance = glyphs[&character].1 * size / 1000.0;
        if advance > usable {
            return Err(invalid(
                "The text box is too narrow for the selected font size.",
            ));
        }
        if used + advance > usable && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            used = 0.0;
        }
        current.push(character);
        used += advance;
    }
    lines.push(current);
    let ascent = f32::from(face.ascender()) * unit * size / 1000.0;
    let descent = f32::from(face.descender()).abs() * unit * size / 1000.0;
    let leading = size * 1.4;
    if ascent + descent + (lines.len() - 1) as f32 * leading + 8.0 > local_height {
        return Err(invalid(
            "The text does not fit this box. Use fewer lines, a smaller size, or place it higher on the page.",
        ));
    }
    let (font, encoded) = embed_text_font(document, text)?;
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec![Object::Name(b"FolioFont".to_vec()), size.into()]),
        Operation::new("rg", color.iter().map(|value| (*value).into()).collect()),
        Operation::new("TL", vec![leading.into()]),
        Operation::new("Td", vec![4.0.into(), (local_height - 4.0 - ascent).into()]),
    ];
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            operations.push(Operation::new("T*", vec![]));
        }
        let bytes: Vec<_> = line
            .chars()
            .flat_map(|character| encoded[&character].to_be_bytes())
            .collect();
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(bytes, StringFormat::Hexadecimal)],
        ));
    }
    operations.extend([Operation::new("ET", vec![]), Operation::new("Q", vec![])]);
    Ok(document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => numbers(&[0.0, 0.0, local_width, local_height]), "Matrix" => numbers(&matrix),
            "Resources" => dictionary! { "Font" => dictionary! { "FolioFont" => font } },
        },
        Content { operations }.encode()?,
    )))
}

pub(crate) fn embedded_text_glyph(
    face: &ttf_parser::Face<'_>,
    character: char,
) -> Result<ttf_parser::GlyphId> {
    let code = character as u32;
    if character.is_control()
        || !matches!(code, 0x20..=0x24f | 0x370..=0x52f | 0x2000..=0x206f | 0x20a0..=0x20cf)
    {
        return Err(invalid(format!(
            "The embedded Noto Sans font supports Latin, Greek, and Cyrillic text (unsupported U+{code:04X})."
        )));
    }
    face.glyph_index(character)
        .ok_or_else(|| invalid(format!("The bundled font does not contain U+{code:04X}.")))
}

pub(crate) fn embed_text_font(
    document: &mut Document,
    text: &str,
) -> Result<(ObjectId, BTreeMap<char, u16>)> {
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|error| invalid(format!("The bundled text font is invalid: {error}")))?;
    let unit = 1000.0 / f32::from(face.units_per_em());
    let glyphs = font_glyphs(&face, text)?;
    let mut font_file = Stream::new(
        dictionary! { "Length1" => FONT.len() as i64 },
        FONT.to_vec(),
    );
    font_file.compress()?;
    let font_file = document.add_object(font_file);
    let bbox = face.global_bounding_box();
    let descriptor = document.add_object(dictionary! {
        "Type" => "FontDescriptor", "FontName" => "NotoSans", "Flags" => 32,
        "FontBBox" => numbers(&[f32::from(bbox.x_min) * unit, f32::from(bbox.y_min) * unit, f32::from(bbox.x_max) * unit, f32::from(bbox.y_max) * unit]),
        "ItalicAngle" => 0, "Ascent" => f32::from(face.ascender()) * unit,
        "Descent" => f32::from(face.descender()) * unit, "CapHeight" => 714,
        "StemV" => 80, "FontFile2" => font_file,
    });
    let mut encoded = BTreeMap::new();
    let mut widths = Vec::new();
    let mut gid_map = vec![0_u8; (glyphs.len() + 1) * 2];
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /FolioUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<_> = glyphs.iter().enumerate().collect();
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (index, (character, (gid, advance))) in chunk {
            let cid = (index + 1) as u16;
            encoded.insert(**character, cid);
            gid_map[usize::from(cid) * 2..usize::from(cid) * 2 + 2]
                .copy_from_slice(&gid.to_be_bytes());
            widths.push(Object::Integer(i64::from(cid)));
            widths.push(Object::Array(vec![Object::Real(*advance)]));
            cmap.push_str(&format!("<{cid:04X}> <{:04X}>\n", **character as u32));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let to_unicode = document.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
    let gid_map = document.add_object(Stream::new(dictionary! {}, gid_map));
    let descendant = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "NotoSans",
        "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
        "FontDescriptor" => descriptor, "CIDToGIDMap" => gid_map,
        "DW" => 1000, "W" => Object::Array(widths),
    });
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "NotoSans",
        "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference(descendant)],
        "ToUnicode" => to_unicode,
    });
    Ok((font, encoded))
}

pub(crate) fn ink_annotation(
    document: &mut Document,
    page: ObjectId,
    points: Vec<[f32; 2]>,
    color: [f32; 3],
    width: f32,
    revision: u64,
) -> Result<Dictionary> {
    validate_color(color)?;
    if !(2..=20_000).contains(&points.len()) || !width.is_finite() || !(0.5..=12.0).contains(&width)
    {
        return Err(invalid(
            "A drawing needs 2 to 20,000 points and a stroke width from 0.5 to 12 points.",
        ));
    }
    let bounds = page_bounds(document, page)?;
    let mut rect = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for point in &points {
        validate_point(*point, bounds)?;
        rect[0] = rect[0].min(point[0]);
        rect[1] = rect[1].min(point[1]);
        rect[2] = rect[2].max(point[0]);
        rect[3] = rect[3].max(point[1]);
    }
    if rect[2] - rect[0] < 0.1 && rect[3] - rect[1] < 0.1 {
        return Err(invalid("Draw a visible stroke before saving it."));
    }
    rect = [
        (rect[0] - width).max(bounds[0]),
        (rect[1] - width).max(bounds[1]),
        (rect[2] + width).min(bounds[2]),
        (rect[3] + width).min(bounds[3]),
    ];
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new("w", vec![width.into()]),
        Operation::new("J", vec![1.into()]),
        Operation::new("j", vec![1.into()]),
        Operation::new("RG", color.iter().map(|value| (*value).into()).collect()),
    ];
    for (index, point) in points.iter().enumerate() {
        operations.push(Operation::new(
            if index == 0 { "m" } else { "l" },
            vec![(point[0] - rect[0]).into(), (point[1] - rect[1]).into()],
        ));
    }
    operations.extend([Operation::new("S", vec![]), Operation::new("Q", vec![])]);
    let appearance = document.add_object(Stream::new(dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[0.0, 0.0, rect[2] - rect[0], rect[3] - rect[1]]),
        "Resources" => dictionary! {},
    }, Content { operations }.encode()?));
    Ok(dictionary! {
        "Type" => "Annot", "Subtype" => "Ink", "P" => page, "Rect" => numbers(&rect),
        "InkList" => vec![numbers(&points.into_iter().flatten().collect::<Vec<_>>())],
        "C" => numbers(&color), "BS" => dictionary! { "W" => width, "S" => "S" },
        "F" => 4, "T" => text_string("Folio"), "Contents" => text_string("Freehand drawing"),
        "NM" => text_string(&format!("folio-{revision}-{}", document.max_id + 1)),
        "AP" => dictionary! { "N" => appearance },
    })
}
