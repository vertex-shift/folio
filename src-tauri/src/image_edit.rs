use std::collections::HashSet;

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream,
    content::{Content, Operation},
};
use serde::{Deserialize, Serialize};

use crate::{
    authoring_image::PreparedImage,
    document::{Result, invalid, page_id},
    page_geometry::{PageGeometry, read_box},
    pdf_objects::{
        MAX_EDIT_CONTENT_BYTES, form_resources, install_resource, page_resources, resolve,
        resource, rewrite_operation,
    },
};

const MAX_OPERATIONS: usize = 200_000;
const MAX_IMAGES: usize = 2_000;
type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageOccurrence {
    pub path: Vec<usize>,
    pub name: String,
    pub pixel_width: Option<u32>,
    pub pixel_height: Option<u32>,
    pub placement: Option<[f32; 4]>,
    pub editable: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePage {
    pub images: Vec<ImageOccurrence>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ImageChange {
    Remove {
        page: usize,
        path: Vec<usize>,
    },
    Place {
        page: usize,
        path: Vec<usize>,
        placement: [f32; 4],
    },
    Replace {
        page: usize,
        path: Vec<usize>,
        asset_id: u64,
        placement: Option<[f32; 4]>,
    },
}

impl ImageChange {
    pub fn asset_id(&self) -> Option<u64> {
        match self {
            Self::Replace { asset_id, .. } => Some(*asset_id),
            _ => None,
        }
    }

    fn target(&self) -> (usize, &[usize], Option<[f32; 4]>) {
        match self {
            Self::Remove { page, path } => (*page, path, None),
            Self::Place {
                page,
                path,
                placement,
            } => (*page, path, Some(*placement)),
            Self::Replace {
                page,
                path,
                placement,
                ..
            } => (*page, path, *placement),
        }
    }
}

#[derive(Clone)]
struct Clip {
    inverse: Matrix,
    bounds: [f32; 4],
}

#[derive(Clone)]
struct Graphics {
    matrix: Matrix,
    block: Option<String>,
    clips: Vec<Clip>,
    text_clip: bool,
}

struct LocatedImage {
    view: ImageOccurrence,
    matrix: Matrix,
    clips: Vec<Clip>,
}

#[derive(Default)]
struct Scanner {
    bytes: usize,
    operations: usize,
    images: Vec<LocatedImage>,
    visiting: HashSet<ObjectId>,
    warnings: Vec<String>,
}

pub fn inspect(document: &Document, page: usize) -> Result<ImagePage> {
    let scanner = scan_page(document, page)?;
    Ok(ImagePage {
        images: scanner.images.into_iter().map(|image| image.view).collect(),
        warnings: scanner.warnings,
    })
}

fn scan_page(document: &Document, page: usize) -> Result<Scanner> {
    let id = page_id(document, page)?;
    let geometry = PageGeometry::for_page(document, id)?;
    let page_dictionary = document.get_dictionary(id)?;
    let mut block = None;
    if page_dictionary.has(b"Group") {
        block = Some("Images in page transparency groups are read-only.".into());
    }
    if document
        .get_pages()
        .values()
        .filter(|value| **value == id)
        .count()
        != 1
    {
        block = Some("This PDF reuses a page object and cannot isolate this page.".into());
    }
    if let Ok(form) = document.catalog()?.get(b"AcroForm")
        && resolve(document, form)?.as_dict()?.has(b"XFA")
    {
        block = Some("Images in dynamic XFA documents cannot be edited safely.".into());
    }
    let graphics = Graphics {
        matrix: IDENTITY,
        block,
        text_clip: false,
        clips: vec![Clip {
            inverse: IDENTITY,
            bounds: geometry.bounds,
        }],
    };
    let mut scanner = Scanner::default();
    scanner.scan(
        document,
        document.get_page_content_with_limit(id, MAX_EDIT_CONTENT_BYTES)?,
        &page_resources(document, id)?,
        graphics,
        &[],
        &geometry,
    )?;
    Ok(scanner)
}

impl Scanner {
    fn scan(
        &mut self,
        document: &Document,
        bytes: Vec<u8>,
        resources: &Dictionary,
        mut graphics: Graphics,
        prefix: &[usize],
        geometry: &PageGeometry,
    ) -> Result<()> {
        self.bytes += bytes.len();
        if self.bytes > MAX_EDIT_CONTENT_BYTES || prefix.len() > 16 {
            return Err(invalid(
                "Image inspection is limited to 16 MiB of decoded content and 16 nested Forms per page.",
            ));
        }
        let content = Content::decode(&bytes)
            .map_err(|_| invalid("This page contains malformed or unsupported content, possibly inline image data. Its images cannot be edited safely."))?;
        self.operations += content.operations.len();
        if self.operations > MAX_OPERATIONS {
            return Err(invalid(
                "Image inspection is limited to 200,000 drawing operations per page.",
            ));
        }
        if content
            .operations
            .iter()
            .any(|op| matches!(op.operator.as_str(), "BI" | "ID" | "EI"))
        {
            graphics.block = Some(
                "This source stream contains inline images and cannot be rewritten safely.".into(),
            );
            if self.warnings.is_empty() {
                self.warnings.push("Inline images are not selectable or editable. Image-XObjects in the same source stream are read-only.".into());
            }
        }
        let mut stack = Vec::new();
        let mut marks: Vec<Option<String>> = Vec::new();
        let mut in_text = false;
        for (index, operation) in content.operations.iter().enumerate() {
            let operand = |index| {
                operation.operands.get(index).ok_or_else(|| {
                    invalid(format!(
                        "The PDF {} operation is incomplete.",
                        operation.operator
                    ))
                })
            };
            match operation.operator.as_str() {
                "q" => {
                    if stack.len() >= 128 {
                        return Err(invalid(
                            "The image graphics-state stack exceeds 128 levels.",
                        ));
                    }
                    stack.push(graphics.clone());
                }
                "Q" => {
                    graphics = stack
                        .pop()
                        .ok_or_else(|| invalid("The page has an unbalanced graphics state."))?
                }
                "cm" => graphics.matrix = multiply(graphics.matrix, matrix(&operation.operands)?),
                "W" | "W*" => {
                    graphics.block =
                        Some("Images affected by a source clipping path are read-only.".into())
                }
                "BT" => in_text = true,
                "ET" => in_text = false,
                "Tr" => graphics.text_clip = operand(0)?.as_i64()? >= 4,
                "Tj" | "TJ" | "'" | "\"" if graphics.text_clip => {
                    graphics.block = Some("Images affected by text clipping are read-only.".into());
                }
                "gs" => {
                    let state = resolve(
                        document,
                        resource(document, resources, b"ExtGState", operand(0)?.as_name()?)?,
                    )?
                    .as_dict()?;
                    if unsupported_graphics(document, state)? {
                        graphics.block = Some("Images with source transparency, blending, masks, transfer functions or overprint are read-only.".into());
                    }
                }
                "BMC" | "BDC" => {
                    let mut block = (operand(0)?.as_name()? == b"OC")
                        .then(|| "Images inside optional-content layers are read-only.".into());
                    if operation.operator == "BDC" {
                        let value = operand(1)?;
                        let value = if let Ok(name) = value.as_name() {
                            resource(document, resources, b"Properties", name)?
                        } else {
                            value
                        };
                        let properties = resolve(document, value)?.as_dict()?;
                        if properties.has(b"OC") || properties.has(b"ActualText") {
                            block = Some("Images with optional-content or ActualText properties are read-only.".into());
                        }
                    }
                    if marks.len() >= 128 {
                        return Err(invalid(
                            "The image marked-content stack exceeds 128 levels.",
                        ));
                    }
                    marks.push(block);
                }
                "EMC" => {
                    marks
                        .pop()
                        .ok_or_else(|| invalid("The page has unbalanced marked content."))?;
                }
                "Do" => {
                    if operation.operands.len() != 1 {
                        return Err(invalid("An image/Form invocation has invalid operands."));
                    }
                    let name = operand(0)?.as_name()?;
                    let value = resource(document, resources, b"XObject", name)?;
                    let stream = resolve(document, value)?.as_stream()?;
                    let subtype = resolve(document, stream.dict.get(b"Subtype")?)?.as_name()?;
                    let mut state = graphics.clone();
                    if let Some(block) = marks.iter().flatten().next() {
                        state.block = Some(block.clone());
                    }
                    if stream.dict.has(b"OC") {
                        state.block =
                            Some("Optional-content images and Forms are read-only.".into());
                    }
                    if in_text {
                        state.block = Some(
                            "Image/Form invocations inside text objects are not supported.".into(),
                        );
                    }
                    let mut path = prefix.to_vec();
                    path.push(index);
                    match subtype {
                        b"Image" => self.image(document, stream, name, path, state, geometry)?,
                        b"Form" => {
                            let id = value.as_reference().map_err(|_| {
                                invalid("A Form has no stable indirect object identifier.")
                            })?;
                            if !self.visiting.insert(id) {
                                return Err(invalid("The PDF has a cyclic Form XObject."));
                            }
                            if let Ok(value) = stream.dict.get(b"Matrix") {
                                state.matrix = multiply(
                                    state.matrix,
                                    matrix(resolve(document, value)?.as_array()?)?,
                                );
                            }
                            if stream.dict.has(b"Group")
                                || stream.dict.has(b"Ref")
                                || stream.dict.has(b"OPI")
                            {
                                state.block = Some("Images in transparency groups, reference or OPI Forms are read-only.".into());
                            }
                            let bounds = read_box(document, stream.dict.get(b"BBox")?)?;
                            if let Ok(inverse) = inverse(state.matrix) {
                                state.clips.push(Clip { inverse, bounds });
                            } else {
                                state.block =
                                    Some("This Form has a singular or unstable transform.".into());
                            }
                            self.scan(
                                document,
                                stream.get_plain_content_with_limit(MAX_EDIT_CONTENT_BYTES)?,
                                &form_resources(document, stream, resources)?,
                                state,
                                &path,
                                geometry,
                            )?;
                            self.visiting.remove(&id);
                        }
                        _ => {}
                    }
                }
                "BI" | "ID" | "EI" | "m" | "l" | "c" | "v" | "y" | "h" | "re" | "S" | "s" | "f"
                | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" | "w" | "J" | "j" | "M" | "d"
                | "ri" | "i" | "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" | "G" | "g" | "RG"
                | "rg" | "K" | "k" | "sh" | "Tc" | "Tw" | "Tz" | "TL" | "Tf" | "Ts" | "Td"
                | "TD" | "Tm" | "T*" | "Tj" | "TJ" | "'" | "\"" | "MP" | "DP" | "BX" | "EX"
                | "d0" | "d1" => {}
                _ => {
                    graphics.block =
                        Some("Images following unsupported drawing operators are read-only.".into())
                }
            }
        }
        if !stack.is_empty() || !marks.is_empty() || in_text {
            return Err(invalid(
                "The source content has unbalanced graphics, text or marked-content state.",
            ));
        }
        Ok(())
    }

    fn image(
        &mut self,
        document: &Document,
        stream: &Stream,
        name: &[u8],
        path: Vec<usize>,
        graphics: Graphics,
        geometry: &PageGeometry,
    ) -> Result<()> {
        if self.images.len() >= MAX_IMAGES {
            return Err(invalid(
                "Image inspection is limited to 2,000 occurrences per page.",
            ));
        }
        let dimension = |key| {
            stream
                .dict
                .get(key)
                .ok()
                .and_then(|value| resolve(document, value).ok())
                .and_then(|value| value.as_i64().ok())
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
        };
        let (pixel_width, pixel_height) = (dimension(b"Width"), dimension(b"Height"));
        let placement = display_placement(geometry, graphics.matrix);
        let mut reason = graphics.block;
        if pixel_width.is_none() || pixel_height.is_none() {
            reason = Some("The source image has invalid pixel dimensions.".into());
        }
        if let Ok(mask) = stream.dict.get(b"ImageMask")
            && resolve(document, mask)?.as_bool()?
        {
            reason = Some("Stencil image masks are read-only.".into());
        }
        if stream.dict.has(b"Alternates") || stream.dict.has(b"OPI") || stream.dict.has(b"F") {
            reason = Some("Alternate, OPI and external-file images are read-only.".into());
        }
        if !axis_aligned(graphics.matrix)
            || inverse(graphics.matrix).is_err()
            || placement.is_none()
        {
            reason = Some("Only finite, nonzero, axis-aligned placements (including right-angle turns and reflections) can be edited; skewed or oblique images are read-only.".into());
        } else if !inside_clips(graphics.matrix, &graphics.clips) {
            reason = Some("The image is clipped by the visible page or a Form BBox. Only completely visible images can be edited.".into());
        }
        self.images.push(LocatedImage {
            view: ImageOccurrence {
                path,
                name: String::from_utf8_lossy(name).chars().take(160).collect(),
                pixel_width,
                pixel_height,
                placement,
                editable: reason.is_none(),
                reason,
            },
            matrix: graphics.matrix,
            clips: graphics.clips,
        });
        Ok(())
    }
}

fn unsupported_graphics(document: &Document, state: &Dictionary) -> Result<bool> {
    for (key, value) in state.iter() {
        let value = resolve(document, value)?;
        match key.as_slice() {
            b"Type" | b"LW" | b"LC" | b"LJ" | b"ML" | b"D" | b"RI" | b"FL" | b"Font" => {}
            b"ca" | b"CA" if number(value)? == 1.0 => {}
            b"BM" if value.as_name().ok() == Some(b"Normal") => {}
            b"SMask" if value.as_name().ok() == Some(b"None") => {}
            b"OP" | b"op" | b"AIS" if !value.as_bool()? => {}
            b"OPM" if value.as_i64()? == 0 => {}
            _ => return Ok(true),
        }
    }
    Ok(false)
}

pub(crate) fn apply(
    document: &mut Document,
    change: &ImageChange,
    replacement: Option<&PreparedImage>,
) -> Result<()> {
    let (page, path, placement) = change.target();
    if path.is_empty() || path.len() > 17 {
        return Err(invalid(
            "Select an image occurrence with a valid operation path.",
        ));
    }
    let image = scan_page(document, page)?
        .images
        .into_iter()
        .find(|image| image.view.path == path)
        .ok_or_else(|| {
            invalid("That image occurrence no longer exists. Inspect the page again.")
        })?;
    if let Some(reason) = image.view.reason {
        return Err(invalid(reason));
    }
    let geometry = PageGeometry::for_page(document, page_id(document, page)?)?;
    let adjustment = if let Some(placement) = placement {
        let bounds = geometry.display_rect(placement)?;
        let original = bounds_for(image.matrix);
        let sx = f64::from(bounds[2] - bounds[0]) / (original[2] - original[0]);
        let sy = f64::from(bounds[3] - bounds[1]) / (original[3] - original[1]);
        let delta = [
            sx,
            0.0,
            0.0,
            sy,
            f64::from(bounds[0]) - sx * original[0],
            f64::from(bounds[1]) - sy * original[1],
        ];
        let target = multiply(delta, image.matrix);
        if !inside_clips(target, &image.clips) {
            return Err(invalid(
                "Keep the complete image inside the visible page and every enclosing Form BBox. This placement would be clipped.",
            ));
        }
        let adjustment = multiply(inverse(image.matrix)?, target);
        validate_matrix(adjustment)?;
        if matches!(change, ImageChange::Place { .. })
            && adjustment
                .iter()
                .zip(IDENTITY)
                .all(|(a, b)| (a - b).abs() < 0.000001)
        {
            return Err(invalid(
                "Move or resize the selected image before applying.",
            ));
        }
        Some(adjustment)
    } else {
        None
    };
    if change.asset_id().is_some() != replacement.is_some() {
        return Err(invalid(
            "The replacement image asset is missing or does not match this action.",
        ));
    }
    rewrite_operation(document, page, path, |document, resources, operation| {
        if operation.operator != "Do" || operation.operands.len() != 1 {
            return Err(invalid("The selected image invocation changed."));
        }
        if matches!(change, ImageChange::Remove { .. }) {
            return Ok(vec![]);
        }
        let mut operation = operation.clone();
        if let Some(replacement) = replacement {
            let id = replacement.embed(document)?;
            let name = install_resource(
                document,
                resources,
                b"XObject",
                "FolioReplacementImage",
                id.into(),
            )?;
            operation.operands[0] = Object::Name(name);
        }
        if let Some(matrix) = adjustment {
            Ok(vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    matrix
                        .iter()
                        .map(|value| Object::Real(*value as f32))
                        .collect(),
                ),
                operation,
                Operation::new("Q", vec![]),
            ])
        } else {
            Ok(vec![operation])
        }
    })
}

fn number(value: &Object) -> Result<f64> {
    let value = f64::from(value.as_float()?);
    if !value.is_finite() {
        return Err(invalid("A source drawing value is not finite."));
    }
    Ok(value)
}

fn matrix(values: &[Object]) -> Result<Matrix> {
    if values.len() != 6 {
        return Err(invalid("A source transform must contain six numbers."));
    }
    let mut result = IDENTITY;
    for (index, value) in values.iter().enumerate() {
        result[index] = number(value)?;
    }
    validate_matrix(result)?;
    Ok(result)
}

fn validate_matrix(matrix: Matrix) -> Result<()> {
    if matrix
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1e9)
    {
        return Err(invalid(
            "The image transform is too large or unstable to edit safely.",
        ));
    }
    Ok(())
}

fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

fn point(matrix: Matrix, [x, y]: [f64; 2]) -> [f64; 2] {
    [
        matrix[0] * x + matrix[2] * y + matrix[4],
        matrix[1] * x + matrix[3] * y + matrix[5],
    ]
}

fn inverse(matrix: Matrix) -> Result<Matrix> {
    validate_matrix(matrix)?;
    let [a, b, c, d, e, f] = matrix;
    let determinant = a * d - b * c;
    if determinant.abs() < 1e-10 {
        return Err(invalid("This image has a singular or unstable transform."));
    }
    let result = [
        d / determinant,
        -b / determinant,
        -c / determinant,
        a / determinant,
        (c * f - d * e) / determinant,
        (b * e - a * f) / determinant,
    ];
    validate_matrix(result)?;
    Ok(result)
}

fn axis_aligned([a, b, c, d, ..]: Matrix) -> bool {
    let epsilon = 1e-8 * a.abs().max(b.abs()).max(c.abs()).max(d.abs()).max(1.0);
    (b.abs() < epsilon && c.abs() < epsilon && a.abs() > epsilon && d.abs() > epsilon)
        || (a.abs() < epsilon && d.abs() < epsilon && b.abs() > epsilon && c.abs() > epsilon)
}

fn corners(matrix: Matrix) -> [[f64; 2]; 4] {
    [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]].map(|value| point(matrix, value))
}

fn bounds_for(matrix: Matrix) -> [f64; 4] {
    corners(matrix).into_iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |bounds, [x, y]| {
            [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x),
                bounds[3].max(y),
            ]
        },
    )
}

fn inside_clips(matrix: Matrix, clips: &[Clip]) -> bool {
    corners(matrix).into_iter().all(|corner| {
        clips.iter().all(|clip| {
            let [x, y] = point(clip.inverse, corner);
            let [left, bottom, right, top] = clip.bounds.map(f64::from);
            x.is_finite()
                && y.is_finite()
                && x >= left - 0.00001
                && y >= bottom - 0.00001
                && x <= right + 0.00001
                && y <= top + 0.00001
        })
    })
}

fn display_placement(geometry: &PageGeometry, matrix: Matrix) -> Option<[f32; 4]> {
    if validate_matrix(matrix).is_err() {
        return None;
    }
    let bounds = corners(matrix)
        .into_iter()
        .map(|value| display_point(geometry, value))
        .fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, [x, y]| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)],
        );
    let placement = [
        bounds[0],
        bounds[1],
        bounds[2] - bounds[0],
        bounds[3] - bounds[1],
    ]
    .map(|value| value as f32);
    placement
        .iter()
        .all(|value| value.is_finite())
        .then_some(placement)
}

fn display_point(geometry: &PageGeometry, [x, y]: [f64; 2]) -> [f64; 2] {
    let [left, bottom, right, top] = geometry.bounds.map(f64::from);
    let unit = f64::from(geometry.user_unit);
    match geometry.rotation {
        0 => [(x - left) * unit, (top - y) * unit],
        90 => [(y - bottom) * unit, (x - left) * unit],
        180 => [(right - x) * unit, (y - bottom) * unit],
        270 => [(top - y) * unit, (right - x) * unit],
        _ => unreachable!("PageGeometry validates right-angle rotations"),
    }
}
