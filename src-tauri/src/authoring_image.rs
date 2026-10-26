use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use crate::document::{Result, invalid};

pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 24_000_000;
const MAX_IMAGE_SIDE: u32 = 8192;

pub(crate) struct PreparedImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
    pub preview_png: Vec<u8>,
}

fn image_error(error: image::ImageError) -> crate::document::Error {
    invalid(format!("The PNG/JPEG image could not be decoded: {error}"))
}

fn validate_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(invalid(
            "Images are limited to 24 megapixels and 8,192 pixels per side.",
        ));
    }
    Ok(())
}

fn jpeg_subset(bytes: &[u8]) -> Result<()> {
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            return Err(invalid("The JPEG has an invalid marker sequence."));
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        let marker = *bytes
            .get(offset)
            .ok_or_else(|| invalid("The JPEG is truncated."))?;
        offset += 1;
        if matches!(marker, 0xd8 | 0xd9 | 0xda | 0x00) {
            return Err(invalid("The JPEG has no supported image frame."));
        }
        let length = bytes
            .get(offset..offset + 2)
            .map(|value| usize::from(u16::from_be_bytes([value[0], value[1]])))
            .ok_or_else(|| invalid("The JPEG is truncated."))?;
        if length < 2
            || offset
                .checked_add(length)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(invalid("The JPEG contains a truncated segment."));
        }
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            if !matches!(marker, 0xc0..=0xc2)
                || length < 8
                || bytes[offset + 2] != 8
                || !matches!(bytes[offset + 7], 1 | 3)
            {
                return Err(invalid(
                    "Use an 8-bit grayscale or RGB JPEG. CMYK, lossless, arithmetic-coded, and higher-bit-depth JPEGs are not supported.",
                ));
            }
            return Ok(());
        }
        offset += length;
    }
    Err(invalid("The JPEG is truncated or has no supported frame."))
}

fn reject_animated_png(bytes: &[u8]) -> Result<()> {
    let mut offset = 8;
    while offset + 12 <= bytes.len() {
        let length =
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("four bytes")) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("The PNG contains a truncated chunk."))?;
        if &bytes[offset + 4..offset + 8] == b"acTL" {
            return Err(invalid(
                "Animated PNGs are not supported. Choose a still PNG or JPEG.",
            ));
        }
        offset = end;
    }
    Ok(())
}

impl PreparedImage {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            return Err(invalid("Choose a PNG or JPEG no larger than 16 MiB."));
        }
        let format = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            reject_animated_png(bytes)?;
            ImageFormat::Png
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            jpeg_subset(bytes)?;
            ImageFormat::Jpeg
        } else {
            return Err(invalid(
                "Only still PNG and 8-bit RGB/grayscale JPEG images are supported.",
            ));
        };
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_SIDE);
        limits.max_image_height = Some(MAX_IMAGE_SIDE);
        limits.max_alloc = Some(96 * 1024 * 1024);
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().map_err(image_error)?;
        let (width, height) = decoder.dimensions();
        validate_dimensions(width, height)?;
        let orientation = decoder.orientation().map_err(image_error)?;
        let mut image = DynamicImage::from_decoder(decoder).map_err(image_error)?;
        image.apply_orientation(orientation);
        let (width, height) = (image.width(), image.height());
        let mut preview = Cursor::new(Vec::new());
        image
            .thumbnail(320, 320)
            .write_to(&mut preview, ImageFormat::Png)
            .map_err(image_error)?;
        let rgba = image.into_rgba8();
        let has_alpha = rgba.pixels().any(|pixel| pixel.0[3] != 255);
        let count = (u64::from(width) * u64::from(height)) as usize;
        let mut rgb = Vec::with_capacity(count * 3);
        let mut alpha = has_alpha.then(|| Vec::with_capacity(count));
        for pixel in rgba.pixels() {
            rgb.extend_from_slice(if pixel.0[3] == 0 {
                &[0, 0, 0]
            } else {
                &pixel.0[..3]
            });
            if let Some(alpha) = &mut alpha {
                alpha.push(pixel.0[3]);
            }
        }
        Ok(Self {
            width,
            height,
            rgb,
            alpha,
            preview_png: preview.into_inner(),
        })
    }

    pub fn byte_length(&self) -> usize {
        self.rgb.len() + self.alpha.as_ref().map_or(0, Vec::len) + self.preview_png.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_image_limits_are_enforced_before_pixel_allocation() {
        assert!(validate_dimensions(6000, 4000).is_ok());
        assert!(validate_dimensions(8192, 1).is_ok());
        for (width, height) in [
            (6000, 4001),
            (8193, 1),
            (1, 8193),
            (0, 1),
            (1, 0),
            (u32::MAX, u32::MAX),
        ] {
            assert!(validate_dimensions(width, height).is_err());
        }
    }

    #[test]
    fn animated_png_and_cmyk_or_higher_depth_jpeg_are_explicitly_rejected() {
        let mut animated = b"\x89PNG\r\n\x1a\n".to_vec();
        animated.extend_from_slice(&[0, 0, 0, 0]);
        animated.extend_from_slice(b"acTL");
        animated.extend_from_slice(&[0; 4]);
        assert!(
            PreparedImage::decode(&animated)
                .err()
                .unwrap()
                .to_string()
                .contains("Animated PNG")
        );
        for (precision, components) in [(8, 4), (12, 3)] {
            let bytes = [
                0xff, 0xd8, 0xff, 0xc0, 0, 8, precision, 0, 1, 0, 1, components,
            ];
            assert!(
                PreparedImage::decode(&bytes)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("8-bit grayscale or RGB JPEG")
            );
        }
    }

    #[test]
    fn indexed_transparency_and_sixteen_bit_grayscale_alpha_become_straight_rgb_and_soft_masks() {
        let mut indexed = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut indexed, 2, 1);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::One);
            encoder.set_palette(vec![255, 0, 0, 0, 255, 0]);
            encoder.set_trns(vec![128, 255]);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0b0100_0000])
                .unwrap();
        }
        let image = PreparedImage::decode(&indexed).unwrap();
        assert_eq!(image.rgb, [255, 0, 0, 0, 255, 0]);
        assert_eq!(image.alpha.unwrap(), [128, 255]);
        let mut grayscale = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut grayscale, 1, 1);
            encoder.set_color(png::ColorType::GrayscaleAlpha);
            encoder.set_depth(png::BitDepth::Sixteen);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 255, 128, 128])
                .unwrap();
        }
        let image = PreparedImage::decode(&grayscale).unwrap();
        assert_eq!(image.rgb, [255, 255, 255]);
        assert_eq!(image.alpha.unwrap(), [128]);
    }

    #[test]
    fn png_exif_orientation_is_applied_before_staging_and_embedding() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 3, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_chunk(
                    png::chunk::eXIf,
                    b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0",
                )
                .unwrap();
            writer.write_image_data(&[255; 3 * 2 * 4]).unwrap();
        }
        let image = PreparedImage::decode(&bytes).unwrap();
        assert_eq!((image.width, image.height), (2, 3));
    }
}
