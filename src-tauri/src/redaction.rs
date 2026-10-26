use std::io::Cursor;

use lopdf::{Document, Object, ObjectId, Stream, content::Content, content::Operation, dictionary};
use serde::{Deserialize, Serialize};

use crate::document::{MAX_DOCUMENT_BYTES, Result, invalid};
use crate::page_geometry::PageGeometry;

pub const MAX_REGIONS: usize = 500;
pub const MAX_PAGE_PNG_BYTES: usize = 64 * 1024 * 1024;
const MAX_PAGE_PIXELS: u64 = 24_000_000;
const MAX_DIMENSION: u32 = 16_384;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub page: usize,
    pub rect: [f64; 4],
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RasterPage {
    pub source_bounds: [f64; 4],
    pub rotation: i32,
    pub user_unit: f64,
    pub width_points: f64,
    pub height_points: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub masks: Vec<PixelRect>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub job_id: u64,
    pub document_id: u64,
    pub revision: u64,
    pub dpi: u16,
    pub pages: Vec<RasterPage>,
}

pub(crate) fn plan(
    source: &Document,
    document_id: u64,
    revision: u64,
    job_id: u64,
    dpi: u16,
    regions: &[Region],
) -> Result<Plan> {
    if ![150, 300].contains(&dpi) {
        return Err(invalid("Choose 150 or 300 DPI for the redacted copy."));
    }
    if regions.is_empty() || regions.len() > MAX_REGIONS {
        return Err(invalid(
            "Mark between 1 and 500 redaction areas before exporting.",
        ));
    }
    if let Ok(form) = source.catalog()?.get(b"AcroForm")
        && source.dereference(form)?.1.as_dict()?.has(b"XFA")
    {
        return Err(invalid(
            "XFA forms cannot be rendered reliably for redaction. Use a static PDF copy first.",
        ));
    }
    let ids: Vec<_> = source.get_pages().into_values().collect();
    if regions.iter().any(|region| region.page >= ids.len()) {
        return Err(invalid(
            "A redaction refers to a page that is no longer in this PDF.",
        ));
    }
    let mut pages = Vec::with_capacity(ids.len());
    for (index, id) in ids.into_iter().enumerate() {
        let geometry = PageGeometry::for_page(source, id)?;
        let bounds = geometry.bounds.map(f64::from);
        let rotation = geometry.rotation;
        let user_unit = f64::from(geometry.user_unit);
        let mut width_points = (bounds[2] - bounds[0]) * user_unit;
        let mut height_points = (bounds[3] - bounds[1]) * user_unit;
        if rotation % 180 != 0 {
            std::mem::swap(&mut width_points, &mut height_points);
        }
        let scale = f64::from(dpi) / 72.0;
        let width = (width_points * scale).ceil();
        let height = (height_points * scale).ceil();
        if !width.is_finite()
            || !height.is_finite()
            || width < 1.0
            || height < 1.0
            || width > f64::from(MAX_DIMENSION)
            || height > f64::from(MAX_DIMENSION)
            || width * height > MAX_PAGE_PIXELS as f64
        {
            return Err(invalid(format!(
                "Page {} exceeds the 24-megapixel or 16,384-pixel redaction limit at {dpi} DPI. Try 150 DPI or a smaller page; resolution will not be lowered silently.",
                index + 1
            )));
        }
        let mut page = RasterPage {
            source_bounds: bounds,
            rotation,
            user_unit,
            width_points,
            height_points,
            pixel_width: width as u32,
            pixel_height: height as u32,
            masks: Vec::new(),
        };
        for region in regions.iter().filter(|region| region.page == index) {
            page.masks.push(pixel_rect(&page, region.rect, dpi)?);
        }
        pages.push(page);
    }
    Ok(Plan {
        job_id,
        document_id,
        revision,
        dpi,
        pages,
    })
}

fn pixel_rect(page: &RasterPage, rect: [f64; 4], dpi: u16) -> Result<PixelRect> {
    let bounds = page.source_bounds;
    if rect.iter().any(|value| !value.is_finite())
        || rect[2] <= rect[0]
        || rect[3] <= rect[1]
        || rect[0] < bounds[0] - 0.01
        || rect[1] < bounds[1] - 0.01
        || rect[2] > bounds[2] + 0.01
        || rect[3] > bounds[3] + 0.01
    {
        return Err(invalid(
            "Redaction areas must be finite, nonempty rectangles inside the visible page.",
        ));
    }
    let point = |x: f64, y: f64| {
        let (x, y) = match page.rotation {
            0 => (x - bounds[0], bounds[3] - y),
            90 => (y - bounds[1], x - bounds[0]),
            180 => (bounds[2] - x, y - bounds[1]),
            270 => (bounds[3] - y, bounds[2] - x),
            _ => unreachable!("page_rotation normalizes right-angle rotations"),
        };
        (
            x * page.user_unit * f64::from(dpi) / 72.0,
            y * page.user_unit * f64::from(dpi) / 72.0,
        )
    };
    let a = point(rect[0], rect[1]);
    let b = point(rect[2], rect[3]);
    // Round outward and add one whole pixel; no antialiased source edge survives inside a mark.
    let left = ((a.0.min(b.0).floor() as i64) - 1).clamp(0, i64::from(page.pixel_width)) as u32;
    let top = ((a.1.min(b.1).floor() as i64) - 1).clamp(0, i64::from(page.pixel_height)) as u32;
    let right = ((a.0.max(b.0).ceil() as i64) + 1).clamp(0, i64::from(page.pixel_width)) as u32;
    let bottom = ((a.1.max(b.1).ceil() as i64) + 1).clamp(0, i64::from(page.pixel_height)) as u32;
    if right <= left || bottom <= top {
        return Err(invalid("A redaction area does not cover any page pixels."));
    }
    Ok(PixelRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

pub(crate) struct Job {
    pub plan: Plan,
    document: Document,
    pages_id: ObjectId,
    page_ids: Vec<Object>,
    compressed_bytes: usize,
}

impl Job {
    pub fn new(plan: Plan) -> Self {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let root = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        let info = document.add_object(dictionary! {
            "Producer" => lopdf::text_string("Folio redacted raster copy")
        });
        document.trailer.set("Root", root);
        document.trailer.set("Info", info);
        Self {
            plan,
            document,
            pages_id,
            page_ids: Vec::new(),
            compressed_bytes: 0,
        }
    }

    pub fn stage(&mut self, index: usize, png: &[u8]) -> Result<()> {
        if index != self.page_ids.len() {
            return Err(invalid(
                "Redaction pages must be rendered exactly once in document order.",
            ));
        }
        let page = self
            .plan
            .pages
            .get(index)
            .ok_or_else(|| invalid("That page is outside the redaction export."))?;
        let mut rgb = decode_rgb(png, page)?;
        for rect in &page.masks {
            for y in rect.y..rect.y + rect.height {
                let offset = (y as usize * page.pixel_width as usize + rect.x as usize) * 3;
                rgb[offset..offset + rect.width as usize * 3].fill(0);
            }
        }
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => page.pixel_width as i64, "Height" => page.pixel_height as i64,
                "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Interpolate" => false,
            },
            rgb,
        );
        image.compress()?;
        if self.compressed_bytes + image.content.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid(
                "The redacted copy exceeds 128 MiB. Try 150 DPI or export a smaller document.",
            ));
        }
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        Object::Real(page.width_points as f32),
                        0.into(),
                        0.into(),
                        Object::Real(page.height_points as f32),
                        0.into(),
                        0.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"PageImage".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()?;
        self.compressed_bytes += image.content.len();
        let image_id = self.document.add_object(image);
        let contents = self
            .document
            .add_object(Stream::new(dictionary! {}, content));
        let page_id = self.document.add_object(dictionary! {
            "Type" => "Page", "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(),
                Object::Real(page.width_points as f32), Object::Real(page.height_points as f32)],
            "Resources" => dictionary! { "XObject" => dictionary! { "PageImage" => image_id } },
            "Contents" => contents,
        });
        self.page_ids.push(page_id.into());
        Ok(())
    }

    pub fn bytes(&mut self) -> Result<Vec<u8>> {
        if self.page_ids.len() != self.plan.pages.len() {
            return Err(invalid(
                "Every page must finish rendering before the redacted copy can be exported.",
            ));
        }
        self.document.objects.insert(self.pages_id, dictionary! {
            "Type" => "Pages", "Kids" => self.page_ids.clone(), "Count" => self.page_ids.len() as i64,
        }.into());
        let mut bytes = Vec::new();
        self.document.save_to(&mut bytes)?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid(
                "The redacted copy exceeds the 128 MiB export limit.",
            ));
        }
        Ok(bytes)
    }
}

fn decode_rgb(bytes: &[u8], page: &RasterPage) -> Result<Vec<u8>> {
    if bytes.len() > MAX_PAGE_PNG_BYTES {
        return Err(invalid("A rendered page exceeds the 64 MiB PNG limit."));
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_limits(png::Limits {
        bytes: MAX_PAGE_PIXELS as usize * 4 + 1024 * 1024,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| invalid(format!("The rendered page PNG is invalid: {error}")))?;
    let header = reader.info();
    if header.width != page.pixel_width || header.height != page.pixel_height {
        return Err(invalid(
            "The rendered page dimensions do not match the reviewed redaction plan.",
        ));
    }
    if header.animation_control.is_some() {
        return Err(invalid("Redaction pages must be static PNG images."));
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= MAX_PAGE_PIXELS as usize * 4)
        .ok_or_else(|| invalid("The rendered page exceeds the pixel decoding limit."))?;
    let mut decoded = vec![0; size];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|error| invalid(format!("The rendered page could not be decoded: {error}")))?;
    if output.bit_depth != png::BitDepth::Eight {
        return Err(invalid("Redaction requires 8-bit RGB or RGBA page images."));
    }
    let pixels = &decoded[..output.buffer_size()];
    match output.color_type {
        png::ColorType::Rgb => Ok(pixels.to_vec()),
        png::ColorType::Rgba => {
            let mut rgb =
                Vec::with_capacity(page.pixel_width as usize * page.pixel_height as usize * 3);
            for pixel in pixels.chunks_exact(4) {
                let alpha = u32::from(pixel[3]);
                for value in &pixel[..3] {
                    rgb.push(((u32::from(*value) * alpha + 255 * (255 - alpha) + 127) / 255) as u8);
                }
            }
            Ok(rgb)
        }
        _ => Err(invalid("Redaction requires RGB or RGBA page images.")),
    }
}
