use std::collections::{BTreeMap, HashSet};

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary, text_string,
};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::{
    authoring_image::PreparedImage,
    document::{
        Result, annotation_entries, append_annotation, inherited, invalid, numbers, page_id,
    },
    page_geometry::PageGeometry,
    writing::{embed_text_font, line_metrics, validate_color},
};

pub use crate::authoring_image::MAX_IMAGE_BYTES;
pub const MAX_STAGED_IMAGE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAsset {
    pub asset_id: u64,
    pub width: u32,
    pub height: u32,
    pub preview_png: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct ImageStaging {
    images: BTreeMap<u64, StagedImage>,
    next_id: u64,
}

struct StagedImage {
    asset_id: u64,
    revision: u64,
    image: PreparedImage,
}

impl ImageStaging {
    pub fn stage(&mut self, document_id: u64, revision: u64, bytes: &[u8]) -> Result<ImageAsset> {
        let image = PreparedImage::decode(bytes)?;
        let used: usize = self
            .images
            .iter()
            .filter(|(id, _)| **id != document_id)
            .map(|(_, staged)| staged.image.byte_length())
            .sum();
        if used + image.byte_length() > MAX_STAGED_IMAGE_BYTES {
            return Err(invalid(
                "Pending images across all PDF tabs exceed 128 MiB of decoded pixels. Place or discard another pending image first.",
            ));
        }
        let asset_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Image identifiers are exhausted. Restart Folio."))?;
        let result = ImageAsset {
            asset_id,
            width: image.width,
            height: image.height,
            preview_png: image.preview_png.clone(),
        };
        self.images.insert(
            document_id,
            StagedImage {
                asset_id,
                revision,
                image,
            },
        );
        self.next_id = asset_id;
        Ok(result)
    }

    pub fn get(&self, document_id: u64, revision: u64, asset_id: u64) -> Result<&PreparedImage> {
        self.images.get(&document_id)
            .filter(|staged| staged.asset_id == asset_id && staged.revision == revision)
            .map(|staged| &staged.image)
            .ok_or_else(|| invalid("This image selection is stale or belongs to another PDF. Discard it and select the image again."))
    }

    pub fn cancel(&mut self, document_id: u64, asset_id: u64) -> Result<bool> {
        if self
            .images
            .get(&document_id)
            .is_some_and(|staged| staged.asset_id != asset_id)
        {
            return Err(invalid(
                "A newer image selection is pending. Refresh this authoring draft.",
            ));
        }
        Ok(self.images.remove(&document_id).is_some())
    }

    pub fn clear(&mut self, document_id: u64) {
        self.images.remove(&document_id);
    }

    pub fn contains(&self, document_id: u64) -> bool {
        self.images.contains_key(&document_id)
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagePlacement {
    pub page: usize,
    pub placement: [f32; 4],
    pub opacity: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatermarkSettings {
    pub text: String,
    pub size: f32,
    pub color: [f32; 3],
    pub opacity: f32,
    pub angle: f32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberPosition {
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageNumberSettings {
    pub start: u32,
    pub prefix: String,
    pub suffix: String,
    pub position: NumberPosition,
    pub size: f32,
    pub color: [f32; 3],
    pub margin: f32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StampKind {
    Watermark,
    PageNumber,
}

impl StampKind {
    fn name(self) -> &'static str {
        match self {
            Self::Watermark => "watermark",
            Self::PageNumber => "page_number",
        }
    }
}

fn static_document(document: &Document) -> Result<()> {
    if let Ok(form) = document.catalog()?.get(b"AcroForm")
        && document.dereference(form)?.1.as_dict()?.has(b"XFA")
    {
        return Err(invalid(
            "Page authoring does not support dynamic XFA forms. Use a static PDF copy.",
        ));
    }
    Ok(())
}

fn chosen_pages(document: &Document, pages: &[usize]) -> Result<Vec<ObjectId>> {
    let all: Vec<_> = document.get_pages().into_values().collect();
    if pages.is_empty()
        || pages.len() > 10_000
        || pages.iter().copied().collect::<HashSet<_>>().len() != pages.len()
        || pages.iter().any(|page| *page >= all.len())
    {
        return Err(invalid(
            "Choose valid, non-repeating pages for this authoring operation.",
        ));
    }
    static_document(document)?;
    Ok(pages.iter().map(|page| all[*page]).collect())
}

fn opacity(value: f32) -> Result<()> {
    if !value.is_finite() || !(0.01..=1.0).contains(&value) {
        return Err(invalid("Opacity must be between 1% and 100%."));
    }
    Ok(())
}

fn size(value: f32) -> Result<()> {
    if !value.is_finite() || !(6.0..=144.0).contains(&value) {
        return Err(invalid("Choose a font size between 6 and 144 points."));
    }
    Ok(())
}

fn single_line(text: &str, max: usize, allow_empty: bool) -> Result<String> {
    let text: String = text.nfc().collect();
    if (!allow_empty && text.trim().is_empty())
        || text.chars().count() > max
        || text.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "Use a single line of at most {max} characters."
        )));
    }
    Ok(text)
}

fn remember_crop(document: &mut Document, page: ObjectId) -> Result<()> {
    if !document.get_dictionary(page)?.has(b"FolioOriginalCropBox") {
        let original = inherited(document, page, b"CropBox")?.unwrap_or(Object::Null);
        document
            .get_dictionary_mut(page)?
            .set("FolioOriginalCropBox", original);
    }
    Ok(())
}

pub(crate) fn crop(document: &mut Document, pages: &[usize], margins: [f32; 4]) -> Result<()> {
    crate::pages::reject_forms(document)?;
    if margins
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        || margins.iter().all(|value| *value == 0.0)
    {
        return Err(invalid(
            "Use non-negative crop margins, with at least one margin greater than zero.",
        ));
    }
    for id in chosen_pages(document, pages)? {
        let geometry = PageGeometry::for_page(document, id)?;
        let [top, right, bottom, left] = margins;
        let width = geometry.width_points - left - right;
        let height = geometry.height_points - top - bottom;
        if width < 1.0 || height < 1.0 {
            return Err(invalid(
                "Crop margins must leave at least one physical point of visible width and height on every chosen page.",
            ));
        }
        let bounds = geometry.display_rect([left, top, width, height])?;
        remember_crop(document, id)?;
        document
            .get_dictionary_mut(id)?
            .set("CropBox", numbers(&bounds));
    }
    Ok(())
}

pub(crate) fn remove_crop(document: &mut Document, pages: &[usize]) -> Result<()> {
    crate::pages::reject_forms(document)?;
    for id in chosen_pages(document, pages)? {
        let geometry = PageGeometry::for_page(document, id)?;
        remember_crop(document, id)?;
        document
            .get_dictionary_mut(id)?
            .set("CropBox", numbers(&geometry.media_box));
    }
    Ok(())
}

pub(crate) fn reset_crop(document: &mut Document, pages: &[usize]) -> Result<()> {
    crate::pages::reject_forms(document)?;
    let mut restored = false;
    for id in chosen_pages(document, pages)? {
        let page = document.get_dictionary(id)?;
        if !page.has(b"FolioOriginalCropBox") {
            continue;
        }
        let original = page.get(b"FolioOriginalCropBox")?.clone();
        let original = if original == Object::Null {
            numbers(&PageGeometry::for_page(document, id)?.media_box)
        } else {
            original
        };
        document.get_dictionary_mut(id)?.set("CropBox", original);
        restored = true;
    }
    if !restored {
        return Err(invalid(
            "There is no Folio crop to reset on the chosen pages.",
        ));
    }
    Ok(())
}

fn graphics_state(opacity: f32) -> Dictionary {
    dictionary! {
        "GS0" => dictionary! { "Type" => "ExtGState", "ca" => opacity, "CA" => opacity },
    }
}

fn appearance(
    document: &mut Document,
    geometry: &PageGeometry,
    dimensions: [f32; 2],
    resources: Dictionary,
    operations: Vec<Operation>,
) -> Result<ObjectId> {
    if matches!(document.version.as_str(), "1.0" | "1.1" | "1.2" | "1.3") {
        document.version = "1.4".into();
    }
    let [width, height] = dimensions;
    let mut form = Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "FormType" => 1,
            "BBox" => numbers(&[0.0, 0.0, width, height]),
            "Matrix" => numbers(&geometry.appearance_matrix(width, height)),
            "Resources" => resources,
        },
        Content { operations }.encode()?,
    );
    form.compress()?;
    Ok(document.add_object(form))
}

fn stamp(
    document: &mut Document,
    page: ObjectId,
    rect: [f32; 4],
    form: ObjectId,
    kind: &str,
    text: &str,
    revision: u64,
) -> Result<()> {
    append_annotation(
        document,
        page,
        dictionary! {
            "Type" => "Annot", "Subtype" => "Stamp", "Name" => "Folio",
            "P" => page, "Rect" => numbers(&rect), "F" => 4,
            "T" => text_string("Folio"), "Contents" => text_string(text),
            "FolioAuthoring" => Object::Name(kind.as_bytes().to_vec()),
            "NM" => text_string(&format!("folio-authoring-{revision}-{}", document.max_id + 1)),
            "AP" => dictionary! { "N" => form },
        },
    )
}

pub(crate) fn insert_image(
    document: &mut Document,
    image: &PreparedImage,
    settings: ImagePlacement,
    revision: u64,
) -> Result<()> {
    static_document(document)?;
    opacity(settings.opacity)?;
    let page = page_id(document, settings.page)?;
    let geometry = PageGeometry::for_page(document, page)?;
    let rect = geometry.display_rect(settings.placement)?;
    let mut image_dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Image",
        "Width" => i64::from(image.width), "Height" => i64::from(image.height),
        "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Interpolate" => true,
    };
    if let Some(alpha) = &image.alpha {
        let mut mask = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => i64::from(image.width), "Height" => i64::from(image.height),
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            alpha.clone(),
        );
        mask.compress()?;
        image_dict.set("SMask", document.add_object(mask));
    }
    let mut pixels = Stream::new(image_dict, image.rgb.clone());
    pixels.compress()?;
    let image_id = document.add_object(pixels);
    let [_, _, width, height] = settings.placement;
    let form = appearance(
        document,
        &geometry,
        [width, height],
        dictionary! {
            "XObject" => dictionary! { "Image0" => image_id },
            "ExtGState" => graphics_state(settings.opacity),
        },
        vec![
            Operation::new("q", vec![]),
            Operation::new("gs", vec![Object::Name(b"GS0".to_vec())]),
            Operation::new(
                "cm",
                vec![
                    width.into(),
                    0.into(),
                    0.into(),
                    height.into(),
                    0.into(),
                    0.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Image0".to_vec())]),
            Operation::new("Q", vec![]),
        ],
    )?;
    stamp(
        document,
        page,
        rect,
        form,
        "image",
        "Inserted PNG/JPEG image",
        revision,
    )
}

fn remove_from_page(document: &mut Document, page: ObjectId, kind: StampKind) -> Result<usize> {
    let mut entries = annotation_entries(document, page)?;
    let before = entries.len();
    let mut kept = Vec::with_capacity(before);
    for entry in entries.drain(..) {
        let annotation = document.dereference(&entry)?.1.as_dict()?;
        if annotation.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Stamp")
            && annotation
                .get(b"FolioAuthoring")
                .and_then(Object::as_name)
                .ok()
                == Some(kind.name().as_bytes())
        {
            continue;
        }
        kept.push(entry);
    }
    let removed = before - kept.len();
    document.get_dictionary_mut(page)?.set("Annots", kept);
    Ok(removed)
}

pub(crate) fn remove_stamps(
    document: &mut Document,
    pages: &[usize],
    kind: StampKind,
) -> Result<()> {
    let mut removed = 0;
    for page in chosen_pages(document, pages)? {
        removed += remove_from_page(document, page, kind)?;
    }
    if removed == 0 {
        return Err(invalid(
            "No matching Folio watermark or page number was found on the chosen pages.",
        ));
    }
    document.prune_objects();
    Ok(())
}

struct TextStamp<'a> {
    font: ObjectId,
    encoded: &'a BTreeMap<char, u16>,
    size: f32,
    color: [f32; 3],
    opacity: f32,
    angle: f32,
}

fn text_form(
    document: &mut Document,
    geometry: &PageGeometry,
    text: &str,
    style: &TextStamp<'_>,
) -> Result<(ObjectId, [f32; 2])> {
    let (width, ascent, descent) = line_metrics(text, style.size)?;
    let angle = style.angle.to_radians();
    let (sin, cos) = angle.sin_cos();
    let corners = [
        [0.0, -descent],
        [width, -descent],
        [0.0, ascent],
        [width, ascent],
    ];
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for [x, y] in corners {
        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
        bounds = [
            bounds[0].min(x),
            bounds[1].min(y),
            bounds[2].max(x),
            bounds[3].max(y),
        ];
    }
    // A small inset keeps antialiased glyph edges inside the appearance box.
    let dimensions = [bounds[2] - bounds[0] + 2.0, bounds[3] - bounds[1] + 2.0];
    let bytes: Vec<_> = text
        .chars()
        .flat_map(|character| style.encoded[&character].to_be_bytes())
        .collect();
    let form = appearance(
        document,
        geometry,
        dimensions,
        dictionary! {
            "Font" => dictionary! { "FolioFont" => style.font },
            "ExtGState" => graphics_state(style.opacity),
        },
        vec![
            Operation::new("q", vec![]),
            Operation::new("gs", vec![Object::Name(b"GS0".to_vec())]),
            Operation::new("BT", vec![]),
            Operation::new(
                "Tf",
                vec![Object::Name(b"FolioFont".to_vec()), style.size.into()],
            ),
            Operation::new(
                "rg",
                style.color.iter().map(|value| (*value).into()).collect(),
            ),
            Operation::new(
                "Tm",
                vec![
                    cos.into(),
                    sin.into(),
                    (-sin).into(),
                    cos.into(),
                    (1.0 - bounds[0]).into(),
                    (1.0 - bounds[1]).into(),
                ],
            ),
            Operation::new("Tj", vec![Object::String(bytes, StringFormat::Hexadecimal)]),
            Operation::new("ET", vec![]),
            Operation::new("Q", vec![]),
        ],
    )?;
    Ok((form, dimensions))
}

pub(crate) fn watermark(
    document: &mut Document,
    pages: &[usize],
    settings: WatermarkSettings,
    revision: u64,
) -> Result<()> {
    let text = single_line(&settings.text, 160, false)?;
    size(settings.size)?;
    opacity(settings.opacity)?;
    validate_color(settings.color)?;
    if !settings.angle.is_finite() || !(-90.0..=90.0).contains(&settings.angle) {
        return Err(invalid(
            "Choose a watermark angle between -90 and 90 degrees.",
        ));
    }
    let pages = chosen_pages(document, pages)?;
    let (font, encoded) = embed_text_font(document, &text)?;
    let style = TextStamp {
        font,
        encoded: &encoded,
        size: settings.size,
        color: settings.color,
        opacity: settings.opacity,
        angle: settings.angle,
    };
    for page in pages {
        let geometry = PageGeometry::for_page(document, page)?;
        let (form, [width, height]) = text_form(document, &geometry, &text, &style)?;
        let rect = geometry.display_rect([
            (geometry.width_points - width) / 2.0,
            (geometry.height_points - height) / 2.0,
            width,
            height,
        ])?;
        remove_from_page(document, page, StampKind::Watermark)?;
        stamp(document, page, rect, form, "watermark", &text, revision)?;
    }
    document.prune_objects();
    Ok(())
}

pub(crate) fn page_numbers(
    document: &mut Document,
    pages: &[usize],
    settings: PageNumberSettings,
    revision: u64,
) -> Result<()> {
    let prefix = single_line(&settings.prefix, 80, true)?;
    let suffix = single_line(&settings.suffix, 80, true)?;
    size(settings.size)?;
    validate_color(settings.color)?;
    if !settings.margin.is_finite() || !(0.0..=720.0).contains(&settings.margin) {
        return Err(invalid(
            "Choose a page-number margin between 0 and 720 points.",
        ));
    }
    let pages = chosen_pages(document, pages)?;
    if settings.start > 999_999 || u64::from(settings.start) + pages.len() as u64 - 1 > 999_999 {
        return Err(invalid("Page numbering must stay between 0 and 999,999."));
    }
    let (font, encoded) = embed_text_font(document, &format!("{prefix}0123456789{suffix}"))?;
    let style = TextStamp {
        font,
        encoded: &encoded,
        size: settings.size,
        color: settings.color,
        opacity: 1.0,
        angle: 0.0,
    };
    for (index, page) in pages.into_iter().enumerate() {
        let text = format!("{prefix}{}{suffix}", settings.start + index as u32);
        let geometry = PageGeometry::for_page(document, page)?;
        let (form, [width, height]) = text_form(document, &geometry, &text, &style)?;
        let x = match settings.position {
            NumberPosition::TopLeft | NumberPosition::BottomLeft => settings.margin,
            NumberPosition::TopCenter | NumberPosition::BottomCenter => {
                (geometry.width_points - width) / 2.0
            }
            NumberPosition::TopRight | NumberPosition::BottomRight => {
                geometry.width_points - settings.margin - width
            }
        };
        let y = match settings.position {
            NumberPosition::TopLeft | NumberPosition::TopCenter | NumberPosition::TopRight => {
                settings.margin
            }
            _ => geometry.height_points - settings.margin - height,
        };
        let rect = geometry.display_rect([x, y, width, height])?;
        remove_from_page(document, page, StampKind::PageNumber)?;
        stamp(document, page, rect, form, "page_number", &text, revision)?;
    }
    document.prune_objects();
    Ok(())
}
