mod font;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary, text_string,
};
use serde::{Deserialize, Serialize};

use crate::{
    document::{Result, invalid, page_id},
    page_geometry::{PageGeometry, read_box},
    pdf_objects::{
        MAX_EDIT_CONTENT_BYTES, form_resources, install_resource, page_resources, resolve,
        resource, rewrite_range, text,
    },
    text_edit::{ReplacementFont, requires_shaping},
};
use font::{Font, Glyph, Measured};

pub const MAX_PARAGRAPH_CHARACTERS: usize = 8_000;
const MAX_BLOCKS: usize = 1_000;
const MAX_LINES: usize = 500;
const MAX_OPERATIONS: usize = 200_000;
const MAX_PREVIEW_BYTES: usize = 2 * 1024 * 1024;
type Matrix = [f64; 6];
type FontBinding = std::result::Result<Arc<Font>, String>;
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLine {
    pub text: String,
    pub baseline: [f64; 2],
    pub bounds: [f64; 4],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphBlock {
    pub id: String,
    pub path: Vec<usize>,
    pub text: Option<String>,
    pub source_lines: Vec<SourceLine>,
    pub placement: Option<[f32; 4]>,
    pub rotation: i32,
    pub size: f64,
    pub leading: f64,
    pub alignment: Alignment,
    pub font: String,
    pub font_mode: ReplacementFont,
    pub original_font_available: bool,
    pub font_reason: Option<String>,
    pub editable: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphPage {
    pub epoch: u64,
    pub blocks: Vec<ParagraphBlock>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParagraphChange {
    pub epoch: u64,
    pub page: usize,
    pub path: Vec<usize>,
    pub block_id: String,
    pub expected: String,
    pub text: String,
    pub placement: [f32; 4],
    pub size: f64,
    pub leading: f64,
    pub alignment: Alignment,
    pub font: ReplacementFont,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutLine {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub hard_break: bool,
    pub baseline: [f64; 2],
    pub advance: f64,
    pub ink_bounds: Option<[f64; 4]>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphLayout {
    pub width: f64,
    pub height: f64,
    pub used_width: f64,
    pub used_height: f64,
    pub lines: Vec<LayoutLine>,
    pub fits: bool,
    pub overflow: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParagraphPreview {
    pub epoch: u64,
    pub layout: ParagraphLayout,
    pub render_pdf: Option<Vec<u8>>,
}

#[derive(Clone)]
struct Clip {
    inverse: Matrix,
    bounds: [f32; 4],
}

#[derive(Clone, PartialEq)]
struct Fill {
    operator: String,
    values: Vec<f64>,
}

impl Default for Fill {
    fn default() -> Self {
        Self {
            operator: "g".into(),
            values: vec![0.0],
        }
    }
}

#[derive(Clone)]
struct Graphics {
    matrix: Matrix,
    font: Option<Arc<Font>>,
    font_error: Option<String>,
    size: f64,
    char_space: f64,
    word_space: f64,
    scale: f64,
    leading: f64,
    rise: f64,
    render: i64,
    fill: Fill,
    block: Option<String>,
    clips: Vec<Clip>,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            matrix: IDENTITY,
            font: None,
            font_error: None,
            size: 0.0,
            char_space: 0.0,
            word_space: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            render: 0,
            fill: Fill::default(),
            block: None,
            clips: Vec::new(),
        }
    }
}

struct LocatedBlock {
    view: ParagraphBlock,
    end: usize,
    managed: bool,
    font: Option<Arc<Font>>,
    original: Vec<u8>,
    container: Matrix,
    fill: Fill,
    clips: Vec<Clip>,
}

#[derive(Default)]
struct Scanner {
    bytes: usize,
    operations: usize,
    blocks: Vec<LocatedBlock>,
    fonts: HashMap<ObjectId, FontBinding>,
    inline_fonts: Vec<(Object, FontBinding)>,
    font_bytes: usize,
    font_mappings: usize,
    visiting: HashSet<ObjectId>,
}

fn number(value: &Object) -> Result<f64> {
    let value = match value {
        Object::Integer(value) => *value as f64,
        Object::Real(value) => f64::from(*value),
        _ => return Err(invalid("A paragraph drawing operand is not a number.")),
    };
    if !value.is_finite() || value.abs() > 1e9 {
        return Err(invalid(
            "A paragraph drawing value is non-finite or unstable.",
        ));
    }
    Ok(value)
}

fn operand(operation: &Operation, index: usize) -> Result<&Object> {
    operation.operands.get(index).ok_or_else(|| {
        invalid(format!(
            "The PDF {} operation is incomplete.",
            operation.operator
        ))
    })
}

fn matrix(values: &[Object]) -> Result<Matrix> {
    if values.len() != 6 {
        return Err(invalid("A paragraph transform needs six finite numbers."));
    }
    let mut result = IDENTITY;
    for (target, value) in result.iter_mut().zip(values) {
        *target = number(value)?;
    }
    Ok(result)
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

fn point(m: Matrix, [x, y]: [f64; 2]) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

fn inverse(m: Matrix) -> Result<Matrix> {
    let determinant = m[0] * m[3] - m[1] * m[2];
    if !determinant.is_finite() || determinant.abs() < 1e-12 {
        return Err(invalid(
            "The paragraph has a singular or unstable source transform.",
        ));
    }
    let result = [
        m[3] / determinant,
        -m[1] / determinant,
        -m[2] / determinant,
        m[0] / determinant,
        (m[2] * m[5] - m[3] * m[4]) / determinant,
        (m[1] * m[4] - m[0] * m[5]) / determinant,
    ];
    if result
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1e9)
    {
        return Err(invalid(
            "The paragraph transform cannot preserve physical-point precision.",
        ));
    }
    Ok(result)
}

fn objects(values: &[f64]) -> Vec<Object> {
    values
        .iter()
        .map(|value| Object::Real(*value as f32))
        .collect()
}

fn display(geometry: &PageGeometry, position: [f64; 2]) -> [f64; 2] {
    geometry.pdf_to_display(position)
}

fn local(rotation: i32, [x, y]: [f64; 2]) -> [f64; 2] {
    match rotation {
        0 => [x, y],
        90 => [y, -x],
        180 => [-x, -y],
        270 => [-y, x],
        _ => unreachable!(),
    }
}

fn oriented(rotation: i32, [x, y]: [f64; 2]) -> [f64; 2] {
    match rotation {
        0 => [x, y],
        90 => [-y, x],
        180 => [-x, -y],
        270 => [y, -x],
        _ => unreachable!(),
    }
}

fn orientation(geometry: &PageGeometry, m: Matrix) -> Result<(i32, f64)> {
    let p = display(geometry, point(m, [0.0, 0.0]));
    let x = display(geometry, point(m, [1.0, 0.0]));
    let y = display(geometry, point(m, [0.0, 1.0]));
    let (a, b, c, d) = (x[0] - p[0], x[1] - p[1], y[0] - p[0], y[1] - p[1]);
    let scale = a.hypot(b);
    if !scale.is_finite()
        || !(1e-6..=1e6).contains(&scale)
        || (c.hypot(d) - scale).abs() > scale * 1e-5
        || (a * c + b * d).abs() > scale * scale * 1e-5
        || a * d - b * c >= 0.0
    {
        return Err(invalid(
            "Only uniform, unreflected right-angle paragraph transforms are supported; skewed, oblique or stretched source text is read-only.",
        ));
    }
    let direction = [a / scale, b / scale];
    let rotation = if direction[0] > 1.0 - 1e-6 && direction[1].abs() < 1e-6 {
        0
    } else if direction[1] > 1.0 - 1e-6 && direction[0].abs() < 1e-6 {
        90
    } else if direction[0] < -1.0 + 1e-6 && direction[1].abs() < 1e-6 {
        180
    } else if direction[1] < -1.0 + 1e-6 && direction[0].abs() < 1e-6 {
        270
    } else {
        return Err(invalid(
            "Oblique paragraph baselines cannot be reflowed safely.",
        ));
    };
    Ok((rotation, scale))
}

fn bounds(points: impl Iterator<Item = [f64; 2]>) -> [f64; 4] {
    let mut result = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for [x, y] in points {
        result[0] = result[0].min(x);
        result[1] = result[1].min(y);
        result[2] = result[2].max(x);
        result[3] = result[3].max(y);
    }
    result
}

fn corners([left, bottom, right, top]: [f64; 4]) -> [[f64; 2]; 4] {
    [[left, bottom], [right, bottom], [right, top], [left, top]]
}

fn inside_clips(transform: Matrix, width: f64, height: f64, clips: &[Clip]) -> bool {
    clips.iter().all(|clip| {
        corners([0.0, 0.0, width, height]).iter().all(|position| {
            let [x, y] = point(clip.inverse, point(transform, *position));
            let [l, b, r, t] = clip.bounds.map(f64::from);
            x >= l - 1e-5 && x <= r + 1e-5 && y >= b - 1e-5 && y <= t + 1e-5
        })
    })
}

fn validate_text(value: &str) -> Result<()> {
    if value.len() > MAX_PARAGRAPH_CHARACTERS * 4
        || value.chars().count() > MAX_PARAGRAPH_CHARACTERS
        || value.chars().any(|character| {
            character.is_control() && character != '\n'
                || matches!(character, '\u{2028}' | '\u{2029}' | '\u{fffe}' | '\u{ffff}')
        })
    {
        return Err(invalid(
            "Use at most 8,000 characters, with Enter for explicit line breaks. Tabs and other control characters are not supported; use spaces.",
        ));
    }
    if requires_shaping(value) {
        return Err(invalid(
            "Paragraph text requiring combining-mark, bidirectional or complex-script shaping is not supported. Use precomposed Latin, Greek or Cyrillic text with Noto Sans.",
        ));
    }
    Ok(())
}

fn extent(glyphs: &[Glyph]) -> (f64, Option<[f64; 4]>) {
    let mut advance = 0.0;
    let mut ink: Option<[f64; 4]> = None;
    for glyph in glyphs {
        if let Some([l, b, r, t]) = glyph.metric.ink {
            let value = [advance + l, b, advance + r, t];
            ink = Some(match ink {
                Some(previous) => [
                    previous[0].min(value[0]),
                    previous[1].min(b),
                    previous[2].max(value[2]),
                    previous[3].max(t),
                ],
                None => value,
            });
        }
        advance += glyph.metric.advance;
    }
    (advance, ink)
}

fn occupied(glyphs: &[Glyph]) -> f64 {
    let (advance, ink) = extent(glyphs);
    ink.map_or(advance, |ink| advance.max(ink[2]) - ink[0].min(0.0))
}

fn breakable(character: char) -> bool {
    character == '-' || character.is_whitespace() && !matches!(character, '\u{a0}' | '\u{202f}')
}

fn layout(
    text: &str,
    measured: Measured,
    width: f64,
    height: f64,
    leading: f64,
    alignment: Alignment,
) -> Result<ParagraphLayout> {
    let glyphs = &measured.glyphs;
    let mut ranges: Vec<(usize, usize, bool)> = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    let mut last_break = None;
    while cursor < glyphs.len() {
        if glyphs[cursor].character == '\n' {
            ranges.push((start, cursor, true));
            cursor += 1;
            start = cursor;
            last_break = None;
        } else {
            if occupied(&glyphs[start..=cursor]) > width + 1e-6 {
                if let Some(end) = last_break.filter(|end| *end > start && *end <= cursor) {
                    ranges.push((start, end, false));
                    start = end;
                    cursor = start;
                    last_break = None;
                    continue;
                }
                if cursor > start
                    && glyphs[cursor].character.is_whitespace()
                    && breakable(glyphs[cursor].character)
                {
                    ranges.push((start, cursor, false));
                    start = cursor;
                    last_break = None;
                    continue;
                }
                // An unbreakable word is kept intact and reported as overflow, never truncated.
            } else if breakable(glyphs[cursor].character) {
                last_break = Some(cursor + 1);
            }
            cursor += 1;
        }
        if ranges.len() >= MAX_LINES {
            return Err(invalid(
                "A paragraph is limited to 500 layout lines. Widen the box or use fewer explicit breaks.",
            ));
        }
    }
    ranges.push((start, glyphs.len(), false));
    let ascent = glyphs
        .iter()
        .filter_map(|glyph| glyph.metric.ink.map(|ink| ink[3]))
        .fold(measured.ascent, f64::max);
    let descent = glyphs
        .iter()
        .filter_map(|glyph| glyph.metric.ink.map(|ink| -ink[1]))
        .fold(measured.descent, f64::max);
    let used_height = ascent + descent + (ranges.len().saturating_sub(1)) as f64 * leading;
    let mut used_width: f64 = 0.0;
    let mut lines = Vec::new();
    for (index, (start, end, hard_break)) in ranges.into_iter().enumerate() {
        let selected = &glyphs[start..end];
        let (advance, ink) = extent(selected);
        let occupied = occupied(selected);
        used_width = used_width.max(occupied);
        let offset = match alignment {
            Alignment::Left => 0.0,
            Alignment::Center => (width - occupied) / 2.0,
            Alignment::Right => width - occupied,
        };
        let x = offset - ink.map_or(0.0, |ink| ink[0].min(0.0));
        let y = ascent + index as f64 * leading;
        let byte_start = glyphs.get(start).map_or(text.len(), |glyph| glyph.start);
        let byte_end = if start == end {
            byte_start
        } else {
            glyphs[end - 1].end
        };
        lines.push(LayoutLine {
            text: text[byte_start..byte_end].into(),
            start: byte_start,
            end: byte_end,
            hard_break,
            baseline: [x, y],
            advance,
            ink_bounds: ink.map(|[l, b, r, t]| [x + l, y - t, r - l, t - b]),
        });
    }
    let overflow = if used_width > width + 0.001 {
        Some(format!(
            "A word or preserved whitespace needs {used_width:.2} pt of width; this box is {width:.2} pt wide. Widen it, change the text or reduce the size. No characters were split or removed."
        ))
    } else if used_height > height + 0.001 {
        Some(format!(
            "The text needs {used_height:.2} pt of height for {} lines, including ink bearings and descenders; this box is {height:.2} pt high. Enlarge it, change leading or shorten the text.",
            lines.len()
        ))
    } else {
        None
    };
    Ok(ParagraphLayout {
        width,
        height,
        used_width,
        used_height,
        lines,
        fits: overflow.is_none(),
        overflow,
    })
}

struct RawLine {
    text: String,
    baseline: [f64; 2],
    down: f64,
    end: f64,
    ink: Option<[f64; 4]>,
}

struct Builder {
    path: Vec<usize>,
    container: Matrix,
    clips: Vec<Clip>,
    reason: Option<String>,
    font: Option<Arc<Font>>,
    fill: Fill,
    size: f64,
    scale: f64,
    rotation: i32,
    origin: Option<[f64; 2]>,
    lines: Vec<RawLine>,
    original: Vec<u8>,
    characters: usize,
}

impl Builder {
    fn new(index: usize, prefix: &[usize], graphics: &Graphics, mark: Option<&String>) -> Self {
        let mut path = prefix.to_vec();
        path.push(index);
        Self {
            path,
            container: graphics.matrix,
            clips: graphics.clips.clone(),
            reason: graphics.block.clone().or_else(|| mark.cloned()),
            font: None,
            fill: graphics.fill.clone(),
            size: 0.0,
            scale: 0.0,
            rotation: 0,
            origin: None,
            lines: Vec::new(),
            original: Vec::new(),
            characters: 0,
        }
    }

    fn glyph(
        &mut self,
        geometry: &PageGeometry,
        graphics: &Graphics,
        tm: Matrix,
        (character, bytes): (char, &[u8]),
        metric: &font::Metric,
        advance: f64,
    ) -> Result<()> {
        if graphics.render != 0 || graphics.rise.abs() > 1e-8 || (graphics.scale - 1.0).abs() > 1e-8
        {
            self.reason = Some("Only visible filled text without text rise or horizontal stretching can be reflowed. OCR, clipping, stroked and raised text are read-only.".into());
        }
        if let Some(reason) = &graphics.block {
            self.reason = Some(reason.clone());
        }
        let font = graphics
            .font
            .as_ref()
            .ok_or_else(|| invalid("The paragraph has no resolved font."))?;
        let transform = multiply(graphics.matrix, tm);
        let (rotation, scale) = orientation(geometry, transform)?;
        let size = graphics.size * scale;
        if !(2.0..=144.0).contains(&size) {
            self.reason =
                Some("Paragraph editing supports physical text sizes from 2 to 144 pt.".into());
        }
        if self.font.is_none() {
            self.font = Some(font.clone());
            self.size = size;
            self.scale = scale;
            self.fill = graphics.fill.clone();
            self.rotation = rotation;
        } else if self
            .font
            .as_ref()
            .is_some_and(|previous| previous.object != font.object)
            || (self.size - size).abs() > 0.001
            || self.fill != graphics.fill
            || self.rotation != rotation
        {
            self.reason = Some("This text object mixes fonts, sizes, colors or orientations. It cannot be treated as one conservative paragraph.".into());
        }
        let baseline = display(geometry, point(transform, [0.0, 0.0]));
        let origin = *self.origin.get_or_insert(baseline);
        let [x, down] = local(
            self.rotation,
            [baseline[0] - origin[0], baseline[1] - origin[1]],
        );
        let new_line = self
            .lines
            .last()
            .is_none_or(|line| (line.down - down).abs() > 0.02);
        if new_line {
            if let Some(previous) = self.lines.last() {
                let gap = down - previous.down;
                if x.abs() > 0.25 || gap < self.size * 0.6 || gap > self.size * 3.0 {
                    self.reason = Some("This text object has irregular baselines, indentation, multiple paragraphs or columns. Select a uniform single-column source object instead.".into());
                }
                if self.lines.len() > 1 {
                    let leading = self.lines[1].down - self.lines[0].down;
                    if (gap - leading).abs() > 0.25_f64.max(leading * 0.01) {
                        self.reason = Some("This text object's line spacing is irregular; grouping it as one paragraph would be ambiguous.".into());
                    }
                }
            }
            self.lines.push(RawLine {
                text: String::new(),
                baseline,
                down,
                end: x,
                ink: None,
            });
        }
        let line = self
            .lines
            .last_mut()
            .ok_or_else(|| invalid("The paragraph line is unavailable."))?;
        let gap = x - line.end;
        if !line.text.is_empty()
            && (gap.abs() > self.size * 0.3
                || gap > self.size * 0.15
                    && !line.text.ends_with(char::is_whitespace)
                    && !character.is_whitespace())
        {
            self.reason = Some("The source uses large unencoded text-position gaps or overlapping runs. Reflow would guess missing whitespace or merge columns, so it is read-only.".into());
        }
        line.text.push(character);
        line.end = x + advance * scale;
        if let Some([l, b, r, t]) = metric.ink {
            let ink = bounds(corners([l, b, r, t]).into_iter().map(|position| {
                let position = display(
                    geometry,
                    point(
                        transform,
                        position.map(|value| value * graphics.size / 1000.0),
                    ),
                );
                local(
                    self.rotation,
                    [position[0] - origin[0], position[1] - origin[1]],
                )
            }));
            line.ink = Some(match line.ink {
                Some(previous) => [
                    previous[0].min(ink[0]),
                    previous[1].min(ink[1]),
                    previous[2].max(ink[2]),
                    previous[3].max(ink[3]),
                ],
                None => ink,
            });
        }
        self.original.extend(bytes);
        self.characters += 1;
        if self.characters > MAX_PARAGRAPH_CHARACTERS || self.lines.len() > MAX_LINES {
            return Err(invalid(
                "A source paragraph exceeds the 8,000-character or 500-line editing limit.",
            ));
        }
        Ok(())
    }

    fn finish(mut self, end: usize, geometry: &PageGeometry) -> Option<LocatedBlock> {
        if self.characters == 0 {
            return None;
        }
        let font = self.font.as_ref();
        let mut text = String::new();
        for line in &self.lines {
            if !text.is_empty()
                && !text.ends_with(char::is_whitespace)
                && !line.text.starts_with(char::is_whitespace)
            {
                text.push(' ');
            }
            text.push_str(&line.text);
        }
        if requires_shaping(&text) || text.chars().any(|character| character.is_control()) {
            self.reason = Some("The source contains shaping-dependent or control characters that cannot be reflowed reliably.".into());
        }
        let leading = if self.lines.len() > 1 {
            self.lines[1].down - self.lines[0].down
        } else {
            self.size * 1.4
        };
        let mut placement = None;
        if let (Some(font), Some(origin), Some(last)) = (font, self.origin, self.lines.last()) {
            let left = self
                .lines
                .iter()
                .filter_map(|line| line.ink.map(|ink| ink[0]))
                .fold(0.0, f64::min);
            let right = self
                .lines
                .iter()
                .map(|line| line.end.max(line.ink.map_or(0.0, |ink| ink[2])))
                .fold(self.size, f64::max);
            let ascent = font.ascent * self.size / 1000.0;
            let descent = font.descent * self.size / 1000.0;
            let display_bounds = bounds(
                corners([left, -ascent, right, last.down + descent])
                    .into_iter()
                    .map(|position| {
                        let [x, y] = oriented(self.rotation, position);
                        [origin[0] + x, origin[1] + y]
                    }),
            );
            let candidate = [
                display_bounds[0] as f32,
                display_bounds[1] as f32,
                (display_bounds[2] - display_bounds[0]) as f32,
                (display_bounds[3] - display_bounds[1]) as f32,
            ];
            if candidate.iter().all(|value| value.is_finite()) {
                placement = Some(candidate);
                if geometry.display_rect(candidate).is_err() {
                    self.reason = Some("The source paragraph is clipped by the visible CropBox. Only completely visible blocks are editable.".into());
                } else if let Ok(transform) = placement_matrix(geometry, candidate, self.rotation) {
                    let (width, height) = local_dimensions(candidate, self.rotation);
                    if !inside_clips(transform, width, height, &self.clips) {
                        self.reason = Some("The source paragraph is clipped by an enclosing Form BBox and cannot be reflowed safely.".into());
                    }
                }
            }
        }
        let source_lines = self
            .lines
            .into_iter()
            .map(|line| {
                let ink = line.ink.unwrap_or([0.0, line.down, line.end, line.down]);
                let origin = self.origin.unwrap_or([0.0, 0.0]);
                let rectangle = bounds(corners(ink).into_iter().map(|position| {
                    let [x, y] = oriented(self.rotation, position);
                    [origin[0] + x, origin[1] + y]
                }));
                SourceLine {
                    text: line.text,
                    baseline: line.baseline,
                    bounds: [
                        rectangle[0],
                        rectangle[1],
                        rectangle[2] - rectangle[0],
                        rectangle[3] - rectangle[1],
                    ],
                }
            })
            .collect();
        let original_font_available = font.is_some_and(|font| font.original_reason.is_none());
        let id = format!(
            "source-{}-{end}",
            self.path
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(".")
        );
        Some(LocatedBlock {
            view: ParagraphBlock {
                id,
                path: self.path,
                text: Some(text),
                source_lines,
                placement,
                rotation: self.rotation,
                size: self.size,
                leading,
                alignment: Alignment::Left,
                font: font.map_or_else(|| "Unresolved font".into(), |font| font.label.clone()),
                font_mode: if original_font_available {
                    ReplacementFont::Original
                } else {
                    ReplacementFont::Noto
                },
                original_font_available,
                font_reason: font.and_then(|font| font.original_reason.clone()),
                editable: self.reason.is_none() && placement.is_some(),
                reason: self.reason,
            },
            end,
            managed: false,
            font: self.font,
            original: self.original,
            container: self.container,
            fill: self.fill,
            clips: self.clips,
        })
    }
}

fn scan_page(document: &Document, page: usize) -> Result<(Scanner, PageGeometry)> {
    let id = page_id(document, page)?;
    let geometry = PageGeometry::for_page(document, id)?;
    let mut graphics = Graphics::default();
    graphics.clips.push(Clip {
        inverse: IDENTITY,
        bounds: geometry.bounds,
    });
    let catalog = document.catalog()?;
    let page_dictionary = document.get_dictionary(id)?;
    if catalog.has(b"StructTreeRoot")
        || page_dictionary.has(b"StructParents")
        || catalog.get(b"MarkInfo").ok().is_some_and(|value| {
            resolve(document, value)
                .and_then(|value| value.as_dict().map_err(Into::into))
                .is_ok_and(|value| value.has(b"Marked"))
        })
    {
        graphics.block = Some("Tagged documents are read-only for paragraph reflow; rewriting source text would require updating their structure tree.".into());
    }
    if page_dictionary.has(b"Group") {
        graphics.block = Some("Paragraphs in page transparency groups are read-only.".into());
    }
    if document
        .get_pages()
        .values()
        .filter(|value| **value == id)
        .count()
        != 1
    {
        graphics.block =
            Some("This PDF reuses a page object and cannot isolate one page safely.".into());
    }
    if let Ok(form) = catalog.get(b"AcroForm")
        && resolve(document, form)?.as_dict()?.has(b"XFA")
    {
        graphics.block = Some("Paragraph reflow is unavailable for dynamic XFA documents.".into());
    }
    let mut scanner = Scanner::default();
    scanner.scan(
        document,
        document.get_page_content_with_limit(id, MAX_EDIT_CONTENT_BYTES)?,
        &page_resources(document, id)?,
        graphics,
        &[],
        &geometry,
    )?;
    Ok((scanner, geometry))
}

impl Scanner {
    fn bind(&mut self, document: &Document, object: &Object) -> Result<Arc<Font>> {
        if let Object::Reference(id) = object {
            if let Some(font) = self.fonts.get(id) {
                return font.clone().map_err(invalid);
            }
        } else if let Some((_, font)) = self.inline_fonts.iter().find(|(value, _)| value == object)
        {
            return font.clone().map_err(invalid);
        }
        if self.fonts.len() + self.inline_fonts.len() >= 128 {
            return Err(invalid(
                "Paragraph inspection is limited to 128 source fonts per page.",
            ));
        }
        let font = Font::load(document, object).and_then(|font| {
            if font.program_bytes > 32 * 1024 * 1024 - self.font_bytes
                || font.mapping_count > 200_000 - self.font_mappings {
                return Err(invalid("Paragraph source fonts exceed the 32 MiB program or 200,000 character-mapping budget."));
            }
            self.font_bytes += font.program_bytes;
            self.font_mappings += font.mapping_count;
            Ok(font)
        }).map_err(|error| error.to_string());
        if let Object::Reference(id) = object {
            self.fonts.insert(*id, font.clone());
        } else {
            self.inline_fonts.push((object.clone(), font.clone()));
        }
        font.map_err(invalid)
    }

    fn set_font(&mut self, document: &Document, object: Result<&Object>, graphics: &mut Graphics) {
        match object.and_then(|object| self.bind(document, object)) {
            Ok(font) => {
                graphics.font = Some(font);
                graphics.font_error = None;
            }
            Err(error) => {
                graphics.font = None;
                graphics.font_error = Some(error.to_string());
            }
        }
    }

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
                "Paragraph inspection is limited to 16 MiB decoded content and 16 nested Forms per page.",
            ));
        }
        let content = Content::decode(&bytes)
            .map_err(|_| invalid("This page has malformed or unsupported source content, possibly inline image data. Paragraphs cannot be rewritten safely."))?;
        self.operations += content.operations.len();
        if self.operations > MAX_OPERATIONS {
            return Err(invalid(
                "Paragraph inspection is limited to 200,000 source operations per page.",
            ));
        }
        if content
            .operations
            .iter()
            .any(|op| matches!(op.operator.as_str(), "BI" | "ID" | "EI"))
        {
            graphics.block = Some("A source stream or enclosing Form contains inline image data and cannot be rewritten safely.".into());
        }
        let mut stack = Vec::new();
        let mut marks: Vec<String> = Vec::new();
        let mut builder: Option<Builder> = None;
        let mut tm = IDENTITY;
        let mut lm = IDENTITY;
        for (index, op) in content.operations.iter().enumerate() {
            let inside = builder.is_some();
            if inside
                && !matches!(
                    op.operator.as_str(),
                    "ET" | "Tf"
                        | "Tc"
                        | "Tw"
                        | "Tz"
                        | "TL"
                        | "Tr"
                        | "Ts"
                        | "Tm"
                        | "Td"
                        | "TD"
                        | "T*"
                        | "Tj"
                        | "TJ"
                        | "'"
                        | "\""
                        | "g"
                        | "rg"
                        | "k"
                )
            {
                builder.as_mut().unwrap().reason = Some("The text object contains non-text drawing, marked content or graphics-state operations; it cannot be replaced as one paragraph.".into());
            }
            match op.operator.as_str() {
                "BT" => {
                    if builder.is_some() { return Err(invalid("The source has nested text objects.")); }
                    builder = Some(Builder::new(index,prefix,&graphics,marks.last()));
                    tm = IDENTITY; lm = IDENTITY;
                }
                "ET" => {
                    let mut current = builder.take().ok_or_else(|| invalid("The source has an unmatched ET operation."))?;
                    if current.characters == 0 && current.reason.is_some() {
                        current.characters = 1;
                    }
                    if let Some(block) = current.finish(index,geometry) { self.blocks.push(block); }
                    if self.blocks.len() > MAX_BLOCKS { return Err(invalid("A page has more than 1,000 paragraph objects.")); }
                }
                "q" => {
                    if stack.len() >= 128 { return Err(invalid("The paragraph graphics stack exceeds 128 levels.")); }
                    stack.push(graphics.clone());
                }
                "Q" => graphics = stack.pop().ok_or_else(|| invalid("The source has an unbalanced graphics state."))?,
                "cm" => graphics.matrix = multiply(graphics.matrix,matrix(&op.operands)?),
                "Tf" => {
                    graphics.size = number(operand(op,1)?)?;
                    self.set_font(document,resource(document,resources,b"Font",operand(op,0)?.as_name()?),&mut graphics);
                }
                "Tc" => graphics.char_space = number(operand(op,0)?)?,
                "Tw" => graphics.word_space = number(operand(op,0)?)?,
                "Tz" => graphics.scale = number(operand(op,0)?)?/100.0,
                "TL" => graphics.leading = number(operand(op,0)?)?,
                "Tr" => graphics.render = operand(op,0)?.as_i64()?,
                "Ts" => graphics.rise = number(operand(op,0)?)?,
                "Tm" => { tm = matrix(&op.operands)?; lm = tm; }
                "Td"|"TD" => {
                    let (x,y) = (number(operand(op,0)?)?,number(operand(op,1)?)?);
                    if op.operator == "TD" {graphics.leading = -y;}
                    lm = multiply(lm,[1.0,0.0,0.0,1.0,x,y]); tm = lm;
                }
                "T*" => {lm = multiply(lm,[1.0,0.0,0.0,1.0,0.0,-graphics.leading]); tm = lm;}
                "Tj"|"TJ"|"'"|"\"" => {
                    let current = builder.as_mut().ok_or_else(|| invalid("The source shows text outside a BT/ET object."))?;
                    if op.operator == "'" || op.operator == "\"" {
                        if op.operator == "\"" {
                            graphics.word_space = number(operand(op,0)?)?;
                            graphics.char_space = number(operand(op,1)?)?;
                        }
                        lm = multiply(lm,[1.0,0.0,0.0,1.0,0.0,-graphics.leading]); tm = lm;
                    }
                    if let Err(error) = show(current,geometry,&graphics,op,&mut tm) {
                        current.reason = Some(error.to_string());
                        // Keep a visible refusal even when the font cannot decode source glyphs.
                        current.characters = current.characters.max(1);
                    }
                }
                "g"|"rg"|"k" => {
                    let count = match op.operator.as_str() {"g"=>1,"rg"=>3,_=>4};
                    if op.operands.len() != count {return Err(invalid("A source text color has invalid operands."));}
                    let values = op.operands.iter().map(number).collect::<Result<Vec<_>>>()?;
                    if values.iter().any(|value| !(0.0..=1.0).contains(value)) {
                        graphics.block = Some("Text colors outside the supported DeviceGray/RGB/CMYK ranges are read-only.".into());
                    }
                    graphics.fill = Fill {operator:op.operator.clone(),values};
                }
                "W"|"W*" => graphics.block = Some("Paragraphs affected by a source clipping path are read-only.".into()),
                "gs" => {
                    let state = resolve(document,resource(document,resources,b"ExtGState",operand(op,0)?.as_name()?)?)?.as_dict()?;
                    if crate::image_edit::unsupported_graphics(document,state)? {
                        graphics.block = Some("Paragraphs affected by transparency, masks, blending, transfer functions or overprint are read-only.".into());
                    }
                    if let Ok(value) = state.get(b"Font") {
                        let value = resolve(document,value)?.as_array()?;
                        if value.len()!=2 {return Err(invalid("An extended graphics-state font is invalid."));}
                        graphics.size = number(&value[1])?;
                        self.set_font(document,Ok(&value[0]),&mut graphics);
                    }
                }
                "BMC"|"BDC" => {
                    let mut reason = "Marked or tagged source text cannot be reflowed without updating its logical structure.".to_owned();
                    if operand(op,0)?.as_name()? == b"OC" {reason = "Paragraphs in optional-content layers are read-only.".into();}
                    if op.operator == "BDC" {
                        let value = operand(op,1)?;
                        let value = if let Ok(name)=value.as_name() {resource(document,resources,b"Properties",name)?} else {value};
                        let properties = resolve(document,value)?.as_dict()?;
                        if properties.has(b"ActualText") {reason = "Paragraphs with ActualText replacement metadata are read-only.".into();}
                    }
                    if marks.len()>=128 {return Err(invalid("The paragraph marked-content stack exceeds 128 levels."));}
                    marks.push(reason);
                }
                "EMC" => {marks.pop().ok_or_else(|| invalid("The source has unbalanced marked content."))?;}
                "Do" => {
                    let object = resource(document,resources,b"XObject",operand(op,0)?.as_name()?)?;
                    let stream = resolve(document,object)?.as_stream()?;
                    if stream.dict.get(b"Subtype")?.as_name()? != b"Form" {continue;}
                    let id = object.as_reference().map_err(|_| invalid("A paragraph Form has no stable indirect identifier."))?;
                    if !self.visiting.insert(id) {return Err(invalid("The source has a cyclic Form invocation."));}
                    let mut state = graphics.clone();
                    if let Some(mark) = marks.last() {state.block = Some(mark.clone());}
                    if inside {state.block = Some("Forms invoked inside text objects are read-only for reflow.".into());}
                    if stream.dict.has(b"Group") || stream.dict.has(b"OC") || stream.dict.has(b"Ref") || stream.dict.has(b"OPI") {
                        state.block = Some("Paragraphs in transparency, optional-content, reference or OPI Forms are read-only.".into());
                    }
                    let mut path = prefix.to_vec(); path.push(index);
                    if stream.dict.has(b"FolioParagraph") {
                        let block = self.managed(document,stream,resources,&state,&path,geometry)?;
                        self.blocks.push(block);
                    } else {
                        if let Ok(value) = stream.dict.get(b"Matrix") {
                            state.matrix = multiply(state.matrix,matrix(resolve(document,value)?.as_array()?)?);
                        }
                        state.clips.push(Clip {inverse:inverse(state.matrix)?,bounds:read_box(document,stream.dict.get(b"BBox")?)?});
                        self.scan(document,stream.get_plain_content_with_limit(MAX_EDIT_CONTENT_BYTES)?,
                            &form_resources(document,stream,resources)?,state,&path,geometry)?;
                    }
                    self.visiting.remove(&id);
                }
                "BI"|"ID"|"EI"|"m"|"l"|"c"|"v"|"y"|"h"|"re"|"S"|"s"|"f"|"F"|"f*"|"B"|"B*"|"b"|"b*"|"n"|
                "w"|"J"|"j"|"M"|"d"|"ri"|"i"|"G"|"RG"|"K"|"sh"|"MP"|"DP"|"BX"|"EX" => {}
                _ => graphics.block = Some("Paragraphs following unsupported drawing or color-space operators are read-only.".into()),
            }
            if graphics.render >= 4 && matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
                graphics.block = Some("Paragraphs affected by text clipping are read-only.".into());
            }
        }
        if !stack.is_empty() || !marks.is_empty() || builder.is_some() {
            return Err(invalid(
                "The source has unbalanced graphics, text or marked-content state.",
            ));
        }
        Ok(())
    }
}

fn show(
    builder: &mut Builder,
    geometry: &PageGeometry,
    graphics: &Graphics,
    operation: &Operation,
    tm: &mut Matrix,
) -> Result<()> {
    if graphics.size <= 0.0 || graphics.scale <= 0.0 {
        return Err(invalid(
            "The source paragraph has an invalid font size or scale.",
        ));
    }
    let font = graphics.font.as_ref().ok_or_else(|| {
        invalid(
            graphics
                .font_error
                .as_deref()
                .unwrap_or("The source font is unresolved."),
        )
    })?;
    let pieces: Vec<&Object> = if operation.operator == "TJ" {
        operand(operation, 0)?.as_array()?.iter().collect()
    } else {
        vec![operand(
            operation,
            if operation.operator == "\"" { 2 } else { 0 },
        )?]
    };
    for piece in pieces {
        if let Object::String(bytes, _) = piece {
            for (decoded, code) in font.codec.units(bytes).map_err(invalid)? {
                let mut characters = decoded.chars();
                let character=characters.next().filter(|_|characters.next().is_none())
                    .ok_or_else(||invalid("Ligatures or source codes mapping to multiple characters cannot be grouped as a paragraph."))?;
                let metric = font.metric(&code, character)?;
                let word_space = if code == [32] {
                    graphics.word_space
                } else {
                    0.0
                };
                let advance =
                    (metric.advance * graphics.size / 1000.0 + graphics.char_space + word_space)
                        * graphics.scale;
                builder.glyph(
                    geometry,
                    graphics,
                    *tm,
                    (character, &code),
                    &metric,
                    advance,
                )?;
                *tm = multiply(*tm, [1.0, 0.0, 0.0, 1.0, advance, 0.0]);
            }
        } else if operation.operator == "TJ" {
            let advance = -number(piece)? / 1000.0 * graphics.size * graphics.scale;
            *tm = multiply(*tm, [1.0, 0.0, 0.0, 1.0, advance, 0.0]);
        } else {
            return Err(invalid(
                "The source text-showing operation has invalid operands.",
            ));
        }
    }
    Ok(())
}

fn local_dimensions(placement: [f32; 4], rotation: i32) -> (f64, f64) {
    if rotation % 180 == 0 {
        (f64::from(placement[2]), f64::from(placement[3]))
    } else {
        (f64::from(placement[3]), f64::from(placement[2]))
    }
}

fn placement_matrix(
    geometry: &PageGeometry,
    [x, y, width, height]: [f32; 4],
    rotation: i32,
) -> Result<Matrix> {
    geometry.display_rect([x, y, width, height])?;
    let anchor = match rotation {
        0 => [x, y + height],
        90 => [x, y],
        180 => [x + width, y],
        270 => [x + width, y + height],
        _ => return Err(invalid("The paragraph orientation is not a right angle.")),
    };
    let anchor = geometry.display_to_pdf_precise(anchor.map(f64::from));
    let scale = 1.0 / f64::from(geometry.user_unit);
    // Display rotation is clockwise, while page rotation maps PDF baselines oppositely.
    let axes = match (rotation - geometry.rotation).rem_euclid(360) {
        0 => [scale, 0.0, 0.0, scale],
        90 => [0.0, -scale, scale, 0.0],
        180 => [-scale, 0.0, 0.0, -scale],
        270 => [0.0, scale, -scale, 0.0],
        _ => unreachable!(),
    };
    Ok([axes[0], axes[1], axes[2], axes[3], anchor[0], anchor[1]])
}

fn text_body(
    font: &Font,
    preferred: &[u8],
    layout: &ParagraphLayout,
    size: f64,
    fill: &Fill,
) -> Result<Vec<Operation>> {
    let mut operations = vec![
        Operation::new("BT", vec![]),
        Operation::new(
            "Tf",
            vec![Object::Name(b"F0".to_vec()), Object::Real(size as f32)],
        ),
        Operation::new("Tc", vec![0.into()]),
        Operation::new("Tw", vec![0.into()]),
        Operation::new("Tz", vec![100.into()]),
        Operation::new("Ts", vec![0.into()]),
        Operation::new("Tr", vec![0.into()]),
        Operation::new(&fill.operator, objects(&fill.values)),
    ];
    for line in &layout.lines {
        if [line.baseline[0], layout.height - line.baseline[1]]
            .iter()
            .any(|value| (f64::from(*value as f32) - value).abs() > 0.001)
        {
            return Err(invalid(
                "The paragraph line coordinates cannot preserve 0.001 physical-point precision. Use a smaller box or less extreme alignment; the source is unchanged.",
            ));
        }
        operations.push(Operation::new(
            "Tm",
            objects(&[
                1.0,
                0.0,
                0.0,
                1.0,
                line.baseline[0],
                layout.height - line.baseline[1],
            ]),
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(
                font.codec.encode(&line.text, preferred)?,
                StringFormat::Hexadecimal,
            )],
        ));
    }
    operations.push(Operation::new("ET", vec![]));
    Ok(operations)
}

fn same_operations(a: &[Operation], b: &[Operation]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.operator == b.operator
                && a.operands.len() == b.operands.len()
                && a.operands
                    .iter()
                    .zip(&b.operands)
                    .all(|(a, b)| match (a, b) {
                        (
                            Object::Integer(_) | Object::Real(_),
                            Object::Integer(_) | Object::Real(_),
                        ) => number(a)
                            .ok()
                            .zip(number(b).ok())
                            .is_some_and(|(a, b)| (a - b).abs() <= 0.001),
                        (Object::String(a, _), Object::String(b, _)) => a == b,
                        _ => a == b,
                    })
        })
}

impl Scanner {
    fn managed(
        &mut self,
        document: &Document,
        stream: &Stream,
        parent: &Dictionary,
        graphics: &Graphics,
        path: &[usize],
        geometry: &PageGeometry,
    ) -> Result<LocatedBlock> {
        let metadata = resolve(document, stream.dict.get(b"FolioParagraph")?)?.as_dict()?;
        if metadata.get(b"V")?.as_i64()? != 1 {
            return Err(invalid(
                "This Folio paragraph uses an unsupported source-block schema.",
            ));
        }
        let id = text(document, metadata.get(b"ID")?, 128)?;
        if !id.starts_with("folio-paragraph-") || id.len() != 48 {
            return Err(invalid("The stored paragraph identity is malformed."));
        }
        let logical = text(
            document,
            metadata.get(b"Text")?,
            MAX_PARAGRAPH_CHARACTERS * 4,
        )?;
        validate_text(&logical)?;
        let size = number(metadata.get(b"Size")?)?;
        let leading = number(metadata.get(b"Leading")?)?;
        validate_style(size, leading)?;
        let alignment = match metadata.get(b"Align")?.as_name()? {
            b"left" => Alignment::Left,
            b"center" => Alignment::Center,
            b"right" => Alignment::Right,
            _ => return Err(invalid("The stored paragraph alignment is unsupported.")),
        };
        let font_mode = match metadata.get(b"FontMode")?.as_name()? {
            b"original" => ReplacementFont::Original,
            b"noto" => ReplacementFont::Noto,
            _ => return Err(invalid("The stored paragraph font choice is unsupported.")),
        };
        let fill = Fill {
            operator: String::from_utf8(metadata.get(b"ColorOp")?.as_name()?.to_vec())
                .map_err(|_| invalid("The stored paragraph color is malformed."))?,
            values: metadata
                .get(b"Color")?
                .as_array()?
                .iter()
                .map(number)
                .collect::<Result<_>>()?,
        };
        let color_len = match fill.operator.as_str() {
            "g" => 1,
            "rg" => 3,
            "k" => 4,
            _ => 0,
        };
        if color_len == 0
            || fill.values.len() != color_len
            || fill.values.iter().any(|value| !(0.0..=1.0).contains(value))
        {
            return Err(invalid("The stored paragraph color is unsupported."));
        }
        let bbox = read_box(document, stream.dict.get(b"BBox")?)?;
        if bbox[0] != 0.0 || bbox[1] != 0.0 {
            return Err(invalid("The stored paragraph has a nonzero local origin."));
        }
        let (width, height) = (f64::from(bbox[2]), f64::from(bbox[3]));
        let resources = form_resources(document, stream, parent)?;
        let font = self.bind(document, resource(document, &resources, b"Font", b"F0")?)?;
        let measured = font::measure(&logical, size, Some((&font, &[])))?;
        let layout = layout(&logical, measured, width, height, leading, alignment)?;
        if !layout.fits {
            return Err(invalid(
                "The stored paragraph overflows its declared source box.",
            ));
        }
        let content = stream.get_plain_content_with_limit(MAX_EDIT_CONTENT_BYTES)?;
        self.bytes += content.len();
        let content = Content::decode(&content)?;
        self.operations += content.operations.len();
        if self.bytes > MAX_EDIT_CONTENT_BYTES
            || self.operations > MAX_OPERATIONS
            || self.blocks.len() >= MAX_BLOCKS
        {
            return Err(invalid(
                "Stored paragraph inspection exceeds the bounded page budget.",
            ));
        }
        let expected = text_body(&font, &[], &layout, size, &fill)?;
        if !same_operations(&content.operations, &expected) {
            return Err(invalid(
                "The stored paragraph metadata no longer matches its real source text and line layout. It cannot be edited safely.",
            ));
        }
        let transform = multiply(
            graphics.matrix,
            matrix(resolve(document, stream.dict.get(b"Matrix")?)?.as_array()?)?,
        );
        let (rotation, scale) = orientation(geometry, transform)?;
        let positions = bounds(
            corners([0.0, 0.0, width, height])
                .into_iter()
                .map(|position| display(geometry, point(transform, position))),
        );
        let placement = [
            positions[0] as f32,
            positions[1] as f32,
            (positions[2] - positions[0]) as f32,
            (positions[3] - positions[1]) as f32,
        ];
        let mut reason = graphics.block.clone();
        if (scale - 1.0).abs() > 1e-5 {
            reason=Some("The stored paragraph's physical scale has changed. Its source layout is read-only rather than guessing new metrics.".into());
        }
        if geometry.display_rect(placement).is_err()
            || !inside_clips(transform, width, height, &graphics.clips)
        {
            reason = Some(
                "The stored paragraph is clipped by the visible page or an enclosing Form BBox."
                    .into(),
            );
        }
        let mut original = Vec::new();
        let source_lines = layout
            .lines
            .iter()
            .map(|line| {
                original.extend(font.codec.encode(&line.text, &[])?);
                let baseline = display(
                    geometry,
                    point(transform, [line.baseline[0], height - line.baseline[1]]),
                );
                let rectangle = line.ink_bounds.unwrap_or([
                    line.baseline[0],
                    line.baseline[1],
                    line.advance,
                    0.0,
                ]);
                let rectangle = bounds(
                    corners([
                        rectangle[0],
                        height - rectangle[1] - rectangle[3],
                        rectangle[0] + rectangle[2],
                        height - rectangle[1],
                    ])
                    .into_iter()
                    .map(|position| display(geometry, point(transform, position))),
                );
                Ok(SourceLine {
                    text: line.text.clone(),
                    baseline,
                    bounds: [
                        rectangle[0],
                        rectangle[1],
                        rectangle[2] - rectangle[0],
                        rectangle[3] - rectangle[1],
                    ],
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(LocatedBlock {
            view: ParagraphBlock {
                id,
                path: path.to_vec(),
                text: Some(logical),
                source_lines,
                placement: Some(placement),
                rotation,
                size,
                leading,
                alignment,
                font: font.label.clone(),
                font_mode,
                original_font_available: true,
                font_reason: None,
                editable: reason.is_none(),
                reason,
            },
            end: *path
                .last()
                .ok_or_else(|| invalid("The paragraph invocation path is empty."))?,
            managed: true,
            font: Some(font),
            original,
            container: graphics.matrix,
            fill,
            clips: graphics.clips.clone(),
        })
    }
}

pub(crate) fn inspect(document: &Document, page: usize, epoch: u64) -> Result<ParagraphPage> {
    let (scanner, _) = scan_page(document, page)?;
    Ok(ParagraphPage {
        epoch,blocks:scanner.blocks.into_iter().map(|block|block.view).collect(),
        warnings:vec!["A source BT/ET text object is grouped only when its baselines form one uniform column. Original visual line endings become wrapping spaces; Enter in the editor inserts an explicit break. Other text, graphics and pages are never moved.".into()],
    })
}

pub(crate) struct SemanticBlock {
    pub view: ParagraphBlock,
    pub end: usize,
    pub managed: bool,
}

pub(crate) fn semantic_blocks(document: &Document, page: usize) -> Result<Vec<SemanticBlock>> {
    let (scanner, _) = scan_page(document, page)?;
    Ok(scanner.blocks.into_iter().map(|block| SemanticBlock {
        view: block.view, end: block.end, managed: block.managed,
    }).collect())
}

fn validate_style(size: f64, leading: f64) -> Result<()> {
    if !size.is_finite()
        || !(2.0..=144.0).contains(&size)
        || !leading.is_finite()
        || !(1.0..=720.0).contains(&leading)
    {
        return Err(invalid(
            "Choose a physical text size from 2 to 144 pt and leading from 1 to 720 pt. Folio never shrinks the font automatically.",
        ));
    }
    Ok(())
}

fn prepare(
    document: &Document,
    change: &ParagraphChange,
) -> Result<(LocatedBlock, PageGeometry, ParagraphLayout)> {
    validate_text(&change.text)?;
    validate_style(change.size, change.leading)?;
    if change.path.is_empty()
        || change.path.len() > 17
        || change.block_id.len() > 256
        || change.expected.len() > MAX_PARAGRAPH_CHARACTERS * 4
    {
        return Err(invalid("Select a current paragraph source occurrence."));
    }
    let (scanner, geometry) = scan_page(document, change.page)?;
    let block = scanner
        .blocks
        .into_iter()
        .find(|block| block.view.path == change.path && block.view.id == change.block_id)
        .ok_or_else(|| {
            invalid("That source paragraph is no longer available. Reload the paragraph list.")
        })?;
    if let Some(reason) = &block.view.reason {
        return Err(invalid(reason));
    }
    if !block.view.editable {
        return Err(invalid("This source paragraph cannot be reflowed safely."));
    }
    if block.view.text.as_deref() != Some(&change.expected) {
        return Err(invalid(
            "The source paragraph changed before this edit. Reload it; the PDF is unchanged.",
        ));
    }
    geometry.display_rect(change.placement)?;
    let transform = placement_matrix(&geometry, change.placement, block.view.rotation)?;
    let (width, height) = local_dimensions(change.placement, block.view.rotation);
    if !inside_clips(transform, width, height, &block.clips) {
        return Err(invalid(
            "Keep the entire paragraph box inside the visible page and all enclosing Form BBoxes. This layout would clip source text.",
        ));
    }
    let font = block
        .font
        .as_ref()
        .ok_or_else(|| invalid("The source paragraph font is unresolved."))?;
    let original = (change.font == ReplacementFont::Original)
        .then_some((font.as_ref(), block.original.as_slice()));
    let measured = font::measure(&change.text, change.size, original)?;
    let layout = layout(
        &change.text,
        measured,
        width,
        height,
        change.leading,
        change.alignment,
    )?;
    Ok((block, geometry, layout))
}

fn form(
    document: &mut Document,
    block: &LocatedBlock,
    change: &ParagraphChange,
    layout: &ParagraphLayout,
    transform: Matrix,
    id: &str,
) -> Result<ObjectId> {
    let source_font = block
        .font
        .as_ref()
        .ok_or_else(|| invalid("The source font is unresolved."))?;
    let font_object = if change.font == ReplacementFont::Original {
        source_font.object.clone()
    } else {
        let text = change.text.replace('\n', "");
        let (font, _) =
            crate::writing::embed_text_font(document, if text.is_empty() { " " } else { &text })?;
        Object::Reference(font)
    };
    let output_font = Font::load(document, &font_object)?;
    let preferred = if change.font == ReplacementFont::Original {
        block.original.as_slice()
    } else {
        &[]
    };
    let operations = text_body(&output_font, preferred, layout, change.size, &block.fill)?;
    let alignment = match change.alignment {
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
    };
    let metadata = dictionary! {
        "V"=>1,"ID"=>text_string(id),"Text"=>text_string(&change.text),
        "Size"=>change.size as f32,"Leading"=>change.leading as f32,"Align"=>alignment,
        "FontMode"=>if change.font==ReplacementFont::Original {"original"} else {"noto"},
        "ColorOp"=>Object::Name(block.fill.operator.as_bytes().to_vec()),"Color"=>objects(&block.fill.values),
    };
    let mut stream = Stream::new(
        dictionary! {
            "Type"=>"XObject","Subtype"=>"Form","FormType"=>1,
            "BBox"=>objects(&[0.0,0.0,layout.width,layout.height]),"Matrix"=>objects(&transform),
            "Resources"=>dictionary! {"Font"=>dictionary! {"F0"=>font_object}},
            "FolioParagraph"=>metadata,
        },
        Content { operations }.encode()?,
    );
    stream.compress()?;
    Ok(document.add_object(stream))
}

pub(crate) fn preview(document: &Document, change: &ParagraphChange) -> Result<ParagraphPreview> {
    let (block, _, layout) = prepare(document, change)?;
    let width = layout.width.max(layout.used_width);
    let height = layout.height.max(layout.used_height);
    let render_pdf = if width <= 20_000.0 && height <= 20_000.0 {
        let mut output = document.clone();
        output.encryption_state = None;
        let mut expanded = layout.clone();
        expanded.width = width;
        expanded.height = height;
        let id = form(
            &mut output,
            &block,
            change,
            &expanded,
            IDENTITY,
            "folio-paragraph-00000000000000000000000000000000",
        )?;
        let content = output.add_object(Stream::new(
            dictionary! {},
            Content {
                operations: vec![
                    Operation::new("Do", vec![Object::Name(b"Paragraph".to_vec())]),
                    Operation::new("RG", objects(&[0.13, 0.35, 0.9])),
                    Operation::new("w", objects(&[0.4])),
                    Operation::new(
                        "re",
                        objects(&[
                            0.2,
                            height - layout.height + 0.2,
                            (layout.width - 0.4).max(0.0),
                            (layout.height - 0.4).max(0.0),
                        ]),
                    ),
                    Operation::new("S", vec![]),
                ],
            }
            .encode()?,
        ));
        let pages = output.new_object_id();
        let page = output.add_object(dictionary! {
            "Type"=>"Page","Parent"=>pages,"MediaBox"=>objects(&[0.0,0.0,width,height]),
            "Contents"=>content,"Resources"=>dictionary! {"XObject"=>dictionary! {"Paragraph"=>id}},
        });
        output.objects.insert(
            pages,
            dictionary! {"Type"=>"Pages","Kids"=>vec![Object::Reference(page)],"Count"=>1}.into(),
        );
        let root = output.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
        output.trailer = dictionary! {"Root"=>root};
        output.prune_objects();
        let bytes = crate::protection::serialize(&output)?;
        if bytes.len() > MAX_PREVIEW_BYTES {
            return Err(invalid(
                "The actual paragraph PDF preview exceeds the 2 MiB limit. No source content changed.",
            ));
        }
        Some(bytes)
    } else {
        None
    };
    Ok(ParagraphPreview {
        epoch: change.epoch,
        layout,
        render_pdf,
    })
}

pub(crate) fn apply(document: &mut Document, change: &ParagraphChange) -> Result<()> {
    let (block, geometry, layout) = prepare(document, change)?;
    if let Some(reason) = &layout.overflow {
        return Err(invalid(format!(
            "Paragraph overflow: {reason} The PDF is unchanged."
        )));
    }
    let desired = placement_matrix(&geometry, change.placement, block.view.rotation)?;
    let transform =
        multiply(inverse(block.container)?, desired).map(|value| f64::from(value as f32));
    let actual = multiply(block.container, transform);
    let (width, height) = local_dimensions(change.placement, block.view.rotation);
    let desired_points = corners([0.0, 0.0, width, height])
        .map(|position| display(&geometry, point(desired, position)));
    let actual_points = corners([0.0, 0.0, width, height])
        .map(|position| display(&geometry, point(actual, position)));
    if actual_points
        .iter()
        .zip(desired_points)
        .any(|(actual, desired)| {
            (actual[0] - desired[0]).abs() > 0.01 || (actual[1] - desired[1]).abs() > 0.01
        })
        || !inside_clips(actual, width, height, &block.clips)
    {
        return Err(invalid(
            "This paragraph transform cannot preserve 0.01 physical-point precision without clipping. Choose a less extreme source geometry; the PDF is unchanged.",
        ));
    }
    let id = if block.managed {
        block.view.id.clone()
    } else {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|error| {
            invalid(format!(
                "A paragraph source identity could not be generated: {error}"
            ))
        })?;
        format!(
            "folio-paragraph-{}",
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    };
    rewrite_range(
        document,
        change.page,
        &change.path,
        block.end,
        |document, resources, selected| {
            let mut replacement = Vec::new();
            if block.managed {
                if selected.len() != 1 || selected[0].operator != "Do" {
                    return Err(invalid("The selected paragraph invocation changed."));
                }
            } else {
                if selected.first().is_none_or(|op| op.operator != "BT")
                    || selected.last().is_none_or(|op| op.operator != "ET")
                {
                    return Err(invalid("The selected paragraph text-object range changed."));
                }
                // Text matrices end at ET, but text/color state survives it. Replay every state effect,
                // including double-quote spacing, without replaying any original source glyphs.
                for operation in selected {
                    match operation.operator.as_str() {
                        "Tj" | "TJ" => {}
                        "'" => replacement.push(Operation::new("T*", vec![])),
                        "\"" => {
                            replacement
                                .push(Operation::new("Tw", vec![operand(operation, 0)?.clone()]));
                            replacement
                                .push(Operation::new("Tc", vec![operand(operation, 1)?.clone()]));
                            replacement.push(Operation::new("T*", vec![]));
                        }
                        _ => replacement.push(operation.clone()),
                    }
                }
            }
            let id = form(document, &block, change, &layout, transform, &id)?;
            let name =
                install_resource(document, resources, b"XObject", "FolioParagraph", id.into())?;
            replacement.push(Operation::new("Do", vec![Object::Name(name)]));
            Ok(replacement)
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_font_bindings_are_cached_and_mapping_budget_is_aggregate() {
        let mut document = Document::with_version("1.7");
        let invalid_font = document.add_object(dictionary! {
            "Type"=>"Font","Subtype"=>"Type3","BaseFont"=>"SyntheticUnsupported",
        });
        let mut scanner = Scanner::default();
        let first = scanner
            .bind(&document, &invalid_font.into())
            .err()
            .unwrap()
            .to_string();
        for _ in 0..20 {
            assert_eq!(
                scanner
                    .bind(&document, &invalid_font.into())
                    .err()
                    .unwrap()
                    .to_string(),
                first
            );
        }
        assert_eq!(scanner.fonts.len(), 1);
        let unicode = document.add_object(Stream::new(dictionary! {},
            b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Synthetic def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange 1 beginbfrange <0000> <FFFF> <0000> endbfrange endcmap CMapName currentdict /CMap defineresource pop end end".to_vec()));
        for index in 0..4 {
            let font = document.add_object(dictionary! {
                "Type"=>"Font","Subtype"=>"Type0","BaseFont"=>"SyntheticCID","Encoding"=>"Identity-H","ToUnicode"=>unicode,
                "DescendantFonts"=>vec![Object::Dictionary(dictionary! {
                    "Type"=>"Font","Subtype"=>"CIDFontType2",
                    "FontDescriptor"=>dictionary! {"FontBBox"=>objects(&[0.0,-200.0,1000.0,800.0])},
                })],
            });
            let result = scanner.bind(&document, &font.into());
            if index < 3 {
                assert!(result.is_ok());
            } else {
                assert!(result.err().unwrap().to_string().contains("200,000"));
                assert!(scanner.bind(&document, &font.into()).is_err());
            }
        }
        assert_eq!(scanner.font_mappings, 3 * 65_536);
        assert_eq!(scanner.fonts.len(), 5);
    }
}
