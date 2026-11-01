use std::{collections::BTreeSet, sync::OnceLock};

use serde::{Deserialize, Serialize};

use crate::document::{Result, invalid};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FontId {
    Latin,
    Arabic,
    Devanagari,
    CjkSc,
    CjkJp,
    CjkKr,
}

impl FontId {
    pub fn key(self) -> &'static str {
        match self {
            Self::Latin => "latin",
            Self::Arabic => "arabic",
            Self::Devanagari => "devanagari",
            Self::CjkSc => "cjk_sc",
            Self::CjkJp => "cjk_jp",
            Self::CjkKr => "cjk_kr",
        }
    }

    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::Latin => crate::writing::FONT,
            Self::Arabic => include_bytes!("../../resources/ocr/fonts/FolioOcrArabic-Regular.ttf"),
            Self::Devanagari => {
                include_bytes!("../../resources/ocr/fonts/FolioOcrDevanagari-Regular.ttf")
            }
            Self::CjkSc => include_bytes!("../../resources/ocr/fonts/FolioOcrCjkSC-Regular.ttf"),
            Self::CjkJp => include_bytes!("../../resources/ocr/fonts/FolioOcrCjkJP-Regular.ttf"),
            Self::CjkKr => include_bytes!("../../resources/ocr/fonts/FolioOcrCjkKR-Regular.ttf"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Language {
    pub id: String,
    pub scripts: Vec<String>,
    pub font: FontId,
    pub bytes: usize,
}

#[derive(Debug, Deserialize)]
pub struct Font {
    pub id: FontId,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub languages: Vec<Language>,
    pub fonts: Vec<Font>,
    pub max_languages: usize,
    pub max_model_bytes: usize,
    pub max_font_bytes: usize,
}

pub fn catalog() -> Result<&'static Catalog> {
    static CATALOG: OnceLock<std::result::Result<Catalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../resources/ocr/catalog.json"))
                .map_err(|error| format!("The bundled OCR catalog is invalid: {error}"))
        })
        .as_ref()
        .map_err(|error| invalid(error.clone()))
}

pub fn selection(languages: &[String]) -> Result<Vec<&'static Language>> {
    let catalog = catalog()?;
    let unique: BTreeSet<_> = languages.iter().collect();
    if languages.is_empty()
        || languages.len() > catalog.max_languages
        || unique.len() != languages.len()
    {
        return Err(invalid(
            "Choose one to three different bundled OCR languages.",
        ));
    }
    let selected = languages.iter().map(|id| catalog.languages.iter().find(|item| item.id == *id)
        .ok_or_else(|| invalid(format!("Unsupported OCR language {id:?}. Choose a bundled model; no English substitution is made."))))
        .collect::<Result<Vec<_>>>()?;
    if selected.iter().map(|item| item.bytes).sum::<usize>() > catalog.max_model_bytes {
        return Err(invalid(
            "The selected OCR models exceed the 20 MiB job limit.",
        ));
    }
    let mut fonts = BTreeSet::from([FontId::Latin]);
    fonts.extend(selected.iter().map(|item| item.font));
    let mut bytes = 0;
    for id in fonts {
        let metadata = catalog
            .fonts
            .iter()
            .find(|font| font.id == id)
            .ok_or_else(|| invalid("A selected OCR font is not in the pinned catalog."))?;
        let data = id.bytes();
        let digest: String = openssl::sha::sha256(data)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if data.len() != metadata.bytes || digest != metadata.sha256 {
            return Err(invalid(format!(
                "The bundled OCR font {} failed SHA-256 verification. No text was added.",
                id.key()
            )));
        }
        let face = ttf_parser::Face::parse(data, 0).map_err(|error| {
            invalid(format!(
                "The bundled OCR font {} is invalid: {error}",
                id.key()
            ))
        })?;
        if face.is_variable()
            || face.tables().glyf.is_none()
            || face.tables().cff.is_some()
            || face.tables().cff2.is_some()
        {
            return Err(invalid(
                "OCR requires the verified static TrueType fonts, never a variable or mislabeled CFF font.",
            ));
        }
        bytes += data.len();
    }
    if bytes > catalog.max_font_bytes {
        return Err(invalid(
            "The selected OCR fonts exceed the 32 MiB job limit.",
        ));
    }
    Ok(selected)
}

pub fn permits(character: char, languages: &[&Language]) -> bool {
    let code = character as u32;
    if matches!(code, 0x21..=0x024f | 0x0300..=0x036f | 0x1e00..=0x1eff
        | 0x2000..=0x206f | 0x20a0..=0x20cf | 0x2100..=0x214f | 0x2200..=0x22ff
        | 0x3000..=0x303f | 0xff00..=0xff65)
    {
        return true;
    }
    let script = match code {
        0x0370..=0x03ff | 0x1f00..=0x1fff => "Greek",
        0x0400..=0x052f => "Cyrillic",
        0x0600..=0x06ff | 0x0750..=0x077f | 0x0870..=0x08ff | 0xfb50..=0xfdff | 0xfe70..=0xfeff => {
            "Arabic"
        }
        0x0900..=0x097f | 0xa8e0..=0xa8ff => "Devanagari",
        0x3040..=0x309f => "Hiragana",
        0x30a0..=0x30ff | 0x31f0..=0x31ff => "Katakana",
        0x1100..=0x11ff | 0x3130..=0x318f | 0xa960..=0xa97f | 0xac00..=0xd7ff => "Hangul",
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af => "Han",
        _ => return false,
    };
    languages
        .iter()
        .any(|item| item.scripts.iter().any(|value| value == script))
}
