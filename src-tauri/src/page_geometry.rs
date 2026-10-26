use lopdf::{Document, Object, ObjectId};
use serde::Serialize;

use crate::document::{Result, inherited, invalid, page_rotation};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageGeometry {
    pub media_box: [f32; 4],
    pub bounds: [f32; 4],
    pub original_bounds: [f32; 4],
    pub rotation: i32,
    pub user_unit: f32,
    pub width_points: f32,
    pub height_points: f32,
    pub crop_modified: bool,
}

pub(crate) fn read_box(document: &Document, value: &Object) -> Result<[f32; 4]> {
    let values = document.dereference(value)?.1.as_array()?;
    if values.len() != 4 {
        return Err(invalid("A page has an invalid crop or media rectangle."));
    }
    let mut bounds = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        bounds[index] = document.dereference(value)?.1.as_float()?;
    }
    if bounds.iter().any(|value| !value.is_finite()) {
        return Err(invalid("A page has non-finite dimensions."));
    }
    if bounds[0] > bounds[2] {
        bounds.swap(0, 2);
    }
    if bounds[1] > bounds[3] {
        bounds.swap(1, 3);
    }
    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return Err(invalid("A page has an empty crop or media rectangle."));
    }
    Ok(bounds)
}

fn intersect(media: [f32; 4], crop: [f32; 4]) -> Result<[f32; 4]> {
    let bounds = [
        media[0].max(crop[0]),
        media[1].max(crop[1]),
        media[2].min(crop[2]),
        media[3].min(crop[3]),
    ];
    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
        return Err(invalid("A page has an empty visible crop rectangle."));
    }
    Ok(bounds)
}

impl PageGeometry {
    pub(crate) fn for_page(document: &Document, id: ObjectId) -> Result<Self> {
        let media = inherited(document, id, b"MediaBox")?
            .ok_or_else(|| invalid("A page has no media dimensions."))?;
        let media_box = read_box(document, &media)?;
        let bounds = match inherited(document, id, b"CropBox")? {
            Some(crop) => intersect(media_box, read_box(document, &crop)?)?,
            None => media_box,
        };
        let page = document.get_dictionary(id)?;
        let original_bounds = match page.get(b"FolioOriginalCropBox") {
            Ok(Object::Null) | Err(lopdf::Error::DictKey(_)) => media_box,
            Ok(value) => intersect(media_box, read_box(document, value)?)?,
            Err(error) => return Err(error.into()),
        };
        let original_bounds = if page.has(b"FolioOriginalCropBox") {
            original_bounds
        } else {
            bounds
        };
        // UserUnit is a page entry, not an inheritable page-tree attribute.
        let user_unit = match page.get(b"UserUnit") {
            Ok(value) => document.dereference(value)?.1.as_float()?,
            Err(lopdf::Error::DictKey(_)) => 1.0,
            Err(error) => return Err(error.into()),
        };
        if !user_unit.is_finite() || user_unit <= 0.0 || user_unit > 75_000.0 {
            return Err(invalid("A page has an invalid physical scale."));
        }
        let rotation = page_rotation(document, id)?;
        let mut width_points = (bounds[2] - bounds[0]) * user_unit;
        let mut height_points = (bounds[3] - bounds[1]) * user_unit;
        if rotation % 180 != 0 {
            std::mem::swap(&mut width_points, &mut height_points);
        }
        if !width_points.is_finite() || !height_points.is_finite() {
            return Err(invalid("A page has invalid physical dimensions."));
        }
        Ok(Self {
            media_box,
            bounds,
            original_bounds,
            rotation,
            user_unit,
            width_points,
            height_points,
            crop_modified: bounds != original_bounds,
        })
    }

    /// Display coordinates are physical points from the visible top-left;
    /// PDF coordinates are unrotated, absolute default-user-space units.
    pub(crate) fn display_to_pdf(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [left, bottom, right, top] = self.bounds;
        let (x, y) = (x / self.user_unit, y / self.user_unit);
        match self.rotation {
            0 => [left + x, top - y],
            90 => [left + y, bottom + x],
            180 => [right - x, bottom + y],
            270 => [right - y, top - x],
            _ => unreachable!("page_rotation normalizes right-angle rotations"),
        }
    }

    pub(crate) fn display_rect(&self, placement: [f32; 4]) -> Result<[f32; 4]> {
        let [x, y, width, height] = placement;
        if placement.iter().any(|value| !value.is_finite())
            || x < 0.0
            || y < 0.0
            || width < 0.1
            || height < 0.1
            || x + width > self.width_points + 0.001
            || y + height > self.height_points + 0.001
        {
            return Err(invalid(
                "Keep the complete placement inside the visible page, with a positive width and height.",
            ));
        }
        let a = self.display_to_pdf([x, y]);
        let b = self.display_to_pdf([x + width, y + height]);
        Ok([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        ])
    }

    pub(crate) fn appearance_matrix(&self, width: f32, height: f32) -> [f32; 6] {
        let scale = 1.0 / self.user_unit;
        match self.rotation {
            0 => [scale, 0.0, 0.0, scale, 0.0, 0.0],
            90 => [0.0, scale, -scale, 0.0, height * scale, 0.0],
            180 => [-scale, 0.0, 0.0, -scale, width * scale, height * scale],
            270 => [0.0, -scale, scale, 0.0, 0.0, width * scale],
            _ => unreachable!("page_rotation normalizes right-angle rotations"),
        }
    }
}
