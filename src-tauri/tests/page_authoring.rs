use std::{collections::BTreeMap, io::Cursor, sync::Arc};

use folio_core::{
    authoring::{ImagePlacement, NumberPosition, PageNumberSettings, StampKind, WatermarkSettings},
    document::{Edit, Session},
    redaction::Region,
    text_edit::ReplacementFont,
    workspace::{OpenOutcome, Workspace},
};
use image::{DynamicImage, ImageFormat, RgbImage};
use lopdf::{
    Dictionary, Document, LoadOptions, Object, Stream,
    content::Content,
    dictionary,
    encryption::{
        EncryptionState, EncryptionVersion, Permissions,
        crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter},
    },
};

fn box_object(values: [f32; 4]) -> Object {
    Object::Array(values.into_iter().map(Object::Real).collect())
}

fn source() -> Vec<u8> {
    let mut pdf = Document::with_version("1.7");
    let root = pdf.new_object_id();
    let branch = pdf.new_object_id();
    let font = pdf.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let mut pages = Vec::new();
    for (index, rotation) in [0, 90, 180, 270].into_iter().enumerate() {
        let stream = pdf.add_object(Stream::new(dictionary! {},
            format!("0.95 0.95 0.95 rg 10 -20 320 460 re f 0 0 0 rg BT /F1 12 Tf 80 200 Td (Original page {}) Tj ET", index + 1).into_bytes()));
        let mut page = dictionary! {
            "Type" => "Page", "Parent" => branch, "Contents" => stream,
            "Rotate" => rotation, "UserUnit" => if index == 1 { 2.0_f32 } else { 1.0_f32 },
        };
        if index == 3 {
            page.set("MediaBox", box_object([-100.0, -50.0, 500.0, 650.0]));
            page.set("CropBox", box_object([-40.0, 0.0, 440.0, 600.0]));
        }
        pages.push(pdf.add_object(page));
    }
    pdf.objects.insert(
        branch,
        dictionary! {
            "Type" => "Pages", "Parent" => root, "Count" => 4,
            "Kids" => pages.into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "MediaBox" => box_object([10.0, -20.0, 330.0, 440.0]),
            "CropBox" => box_object([30.0, 10.0, 310.0, 410.0]),
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        }
        .into(),
    );
    pdf.objects.insert(
        root,
        dictionary! { "Type" => "Pages", "Count" => 4, "Kids" => vec![Object::Reference(branch)] }
            .into(),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    pdf.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("Comment".into(), "PHOTO_LOCATION_SHOULD_NOT_SURVIVE".into())
            .unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 255, 255, 0, 255,
            ])
            .unwrap();
    }
    bytes
}

fn oriented_jpeg() -> Vec<u8> {
    let image = DynamicImage::ImageRgb8(RgbImage::from_fn(3, 2, |x, y| {
        image::Rgb([x as u8 * 70, y as u8 * 90, 80])
    }));
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, ImageFormat::Jpeg).unwrap();
    let encoded = encoded.into_inner();
    // Original synthetic EXIF: little-endian TIFF, orientation 6 (90 degrees CW).
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut bytes = encoded[..2].to_vec();
    bytes.extend_from_slice(&[0xff, 0xe1]);
    bytes.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    bytes.extend_from_slice(exif);
    bytes.extend_from_slice(&encoded[2..]);
    bytes
}

fn workspace() -> Workspace {
    let mut workspace = Workspace::default();
    workspace
        .open_bytes(source(), "Synthetic authoring.pdf".into())
        .unwrap();
    workspace
}

fn watermark(text: &str) -> WatermarkSettings {
    WatermarkSettings {
        text: text.into(),
        size: 24.0,
        color: [0.3, 0.2, 0.6],
        opacity: 0.35,
        angle: 30.0,
    }
}

fn numbers() -> PageNumberSettings {
    PageNumberSettings {
        start: 7,
        prefix: "Page ".into(),
        suffix: " / review".into(),
        position: NumberPosition::BottomRight,
        size: 10.0,
        color: [0.1, 0.2, 0.3],
        margin: 12.0,
    }
}

fn apply(workspace: &mut Workspace, edit: Edit) {
    let info = workspace.current().unwrap().info().unwrap();
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .apply(edit)
        .unwrap();
}

fn pdf(workspace: &Workspace) -> Document {
    Document::load_mem(workspace.current().unwrap().bytes()).unwrap()
}

fn appearance<'a>(pdf: &'a Document, annotation: &'a Dictionary) -> &'a Stream {
    let normal = annotation
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap();
    pdf.dereference(normal).unwrap().1.as_stream().unwrap()
}

fn floats(value: &Object) -> Vec<f32> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_float().unwrap())
        .collect()
}

#[test]
fn crop_uses_visible_physical_edges_on_every_rotation_and_preserves_original_boxes() {
    let mut workspace = workspace();
    let original = workspace.current().unwrap().info().unwrap();
    let content = pdf(&workspace).get_page_content(pdf(&workspace).get_pages()[&1]);
    apply(
        &mut workspace,
        Edit::Crop {
            pages: vec![0, 1, 2, 3],
            margins: [10.0, 20.0, 30.0, 40.0],
        },
    );
    let info = workspace.current().unwrap().info().unwrap();
    let expected = [
        [70.0, 40.0, 290.0, 400.0],
        [35.0, 30.0, 295.0, 400.0],
        [50.0, 20.0, 270.0, 380.0],
        [-10.0, 20.0, 430.0, 560.0],
    ];
    for (index, page) in info.pages.iter().enumerate() {
        assert_eq!(page.geometry.bounds, expected[index]);
        assert_eq!(
            page.geometry.media_box,
            original.pages[index].geometry.media_box
        );
        assert_eq!(
            page.geometry.original_bounds,
            original.pages[index].geometry.bounds
        );
        assert!(page.geometry.crop_modified);
        assert_eq!(
            page.geometry.width_points,
            original.pages[index].geometry.width_points - 60.0
        );
        assert_eq!(
            page.geometry.height_points,
            original.pages[index].geometry.height_points - 40.0
        );
    }
    assert_eq!(
        pdf(&workspace).get_page_content(pdf(&workspace).get_pages()[&1]),
        content
    );
    let saved = workspace.current().unwrap().bytes().to_vec();
    let mut reopened = Session::from_bytes(saved, "Cropped.pdf".into(), None, 10).unwrap();
    reopened.apply(Edit::Move { from: 0, to: 3 }).unwrap();
    reopened
        .apply(Edit::ResetCrop {
            pages: vec![0, 1, 2, 3],
        })
        .unwrap();
    for page in reopened.info().unwrap().pages {
        let source_page = original
            .pages
            .iter()
            .find(|source| source.key == page.key)
            .unwrap();
        assert_eq!(page.geometry.bounds, source_page.geometry.bounds);
        assert!(!page.geometry.crop_modified);
    }
    apply(&mut workspace, Edit::RemoveCrop { pages: vec![0, 1] });
    let info = workspace.current().unwrap().info().unwrap();
    for page in info.pages.iter().take(2) {
        assert_eq!(page.geometry.bounds, page.geometry.media_box);
    }
    apply(&mut workspace, Edit::ResetCrop { pages: vec![0, 1] });
    assert_eq!(
        workspace.current().unwrap().info().unwrap().pages[0]
            .geometry
            .bounds,
        [30.0, 10.0, 310.0, 410.0]
    );
}

#[test]
fn crop_intersects_normalized_inherited_boxes_and_invalid_changes_are_atomic() {
    let mut source = Document::load_mem(&source()).unwrap();
    let parent = source
        .get_dictionary(source.get_pages()[&1])
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    source
        .get_dictionary_mut(parent)
        .unwrap()
        .set("CropBox", box_object([400.0, 430.0, -40.0, -50.0]));
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let mut session = Session::from_bytes(bytes, "Normalized.pdf".into(), None, 1).unwrap();
    assert_eq!(
        session.info().unwrap().pages[0].geometry.bounds,
        [10.0, -20.0, 330.0, 430.0]
    );
    let before = session.bytes().to_vec();
    for edit in [
        Edit::Crop {
            pages: vec![0, 0],
            margins: [1.0; 4],
        },
        Edit::Crop {
            pages: vec![0, 1],
            margins: [1000.0, 0.0, 0.0, 0.0],
        },
        Edit::Crop {
            pages: vec![1, 0],
            margins: [0.0, 200.0, 0.0, 200.0],
        },
        Edit::Crop {
            pages: vec![0],
            margins: [f32::NAN, 0.0, 0.0, 0.0],
        },
        Edit::Crop {
            pages: vec![0],
            margins: [-1.0, 0.0, 0.0, 0.0],
        },
        Edit::Crop {
            pages: vec![0],
            margins: [0.0; 4],
        },
        Edit::ResetCrop { pages: vec![0] },
    ] {
        assert!(session.apply(edit).is_err());
        assert_eq!(session.bytes(), before);
        assert!(!session.info().unwrap().can_undo);
    }
}

#[test]
fn image_stamps_have_real_rgb_pixels_alpha_opacity_orientation_and_no_source_metadata() {
    let mut workspace = workspace();
    for page in 0..4 {
        let info = workspace.current().unwrap().info().unwrap();
        let asset = workspace
            .stage_authoring_image(info.id, info.revision, &png())
            .unwrap();
        assert_eq!((asset.width, asset.height), (2, 2));
        workspace
            .insert_authoring_image(
                info.id,
                info.revision,
                asset.asset_id,
                ImagePlacement {
                    page,
                    placement: [20.0, 40.0, 80.0, 60.0],
                    opacity: 0.5,
                },
            )
            .unwrap();
    }
    let pdf = pdf(&workspace);
    let mut color_images = 0;
    for object in pdf.objects.values() {
        if let Ok(stream) = object.as_stream()
            && stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
            && stream
                .dict
                .get(b"ColorSpace")
                .and_then(Object::as_name)
                .ok()
                == Some(b"DeviceRGB")
        {
            color_images += 1;
            let pixels = stream.get_plain_content().unwrap();
            assert_eq!(pixels, [255, 0, 0, 0, 255, 0, 0, 0, 0, 255, 255, 0]);
            let mask = pdf
                .dereference(stream.dict.get(b"SMask").unwrap())
                .unwrap()
                .1
                .as_stream()
                .unwrap();
            assert_eq!(mask.get_plain_content().unwrap(), [255, 128, 0, 255]);
            for key in [b"Metadata".as_slice(), b"Exif", b"ICCProfile"] {
                assert!(!stream.dict.has(key));
            }
        }
    }
    assert_eq!(color_images, 4);
    for (index, page) in pdf.get_pages().into_values().enumerate() {
        let annotation = pdf.get_page_annotations(page).unwrap()[0];
        assert_eq!(
            annotation.get(b"Subtype").unwrap().as_name().unwrap(),
            b"Stamp"
        );
        let form = appearance(&pdf, annotation);
        assert_eq!(
            floats(form.dict.get(b"BBox").unwrap()),
            [0.0, 0.0, 80.0, 60.0]
        );
        let matrices = [
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [0.0, 0.5, -0.5, 0.0, 30.0, 0.0],
            [-1.0, 0.0, 0.0, -1.0, 80.0, 60.0],
            [0.0, -1.0, 1.0, 0.0, 0.0, 80.0],
        ];
        assert_eq!(floats(form.dict.get(b"Matrix").unwrap()), matrices[index]);
        let state = form
            .dict
            .get(b"Resources")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"ExtGState")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"GS0")
            .unwrap()
            .as_dict()
            .unwrap();
        assert_eq!(state.get(b"ca").unwrap().as_float().unwrap(), 0.5);
        let operations = Content::decode(&form.get_plain_content().unwrap())
            .unwrap()
            .operations;
        assert!(
            operations
                .iter()
                .any(|operation| operation.operator == "Do")
        );
    }
    let info = workspace.current().unwrap().info().unwrap();
    assert_eq!(info.annotations.len(), 4);
    assert!(
        info.annotations
            .iter()
            .all(|annotation| annotation.kind == "image" && annotation.deletable)
    );
    let jpeg = workspace
        .stage_authoring_image(info.id, info.revision, &oriented_jpeg())
        .unwrap();
    assert_eq!((jpeg.width, jpeg.height), (2, 3));
    workspace
        .insert_authoring_image(
            info.id,
            info.revision,
            jpeg.asset_id,
            ImagePlacement {
                page: 0,
                placement: [120.0, 20.0, 40.0, 60.0],
                opacity: 1.0,
            },
        )
        .unwrap();
    let bytes = workspace.current().unwrap().bytes();
    assert!(
        !bytes
            .windows(b"PHOTO_LOCATION".len())
            .any(|value| value == b"PHOTO_LOCATION")
    );
    assert!(!bytes.windows(b"Exif".len()).any(|value| value == b"Exif"));
    let reopened = Session::from_bytes(bytes.to_vec(), "Images.pdf".into(), None, 2).unwrap();
    assert_eq!(reopened.info().unwrap().annotations.len(), 5);
}

#[test]
fn image_selection_is_bounded_document_revision_scoped_and_failed_inserts_keep_the_selection() {
    let mut workspace = workspace();
    let first = workspace.current().unwrap().info().unwrap();
    let before = workspace.current().unwrap().bytes().to_vec();
    let asset = workspace
        .stage_authoring_image(first.id, first.revision, &png())
        .unwrap();
    assert!(workspace.has_unsaved_changes());
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
    assert!(
        workspace
            .close_document(first.id, first.revision, false)
            .is_err()
    );
    let invalid = ImagePlacement {
        page: 0,
        placement: [0.0, 0.0, 900.0, 20.0],
        opacity: 1.0,
    };
    assert!(
        workspace
            .insert_authoring_image(first.id, first.revision, asset.asset_id, invalid)
            .is_err()
    );
    for bad in [
        b"GIF89a".to_vec(),
        vec![0; 16 * 1024 * 1024 + 1],
        b"\x89PNG\r\n\x1a\n".to_vec(),
    ] {
        assert!(
            workspace
                .stage_authoring_image(first.id, first.revision, &bad)
                .is_err()
        );
        assert_eq!(workspace.document(first.id).unwrap().bytes(), before);
    }
    let other = workspace.create_document(612.0, 792.0).unwrap();
    assert!(
        workspace
            .insert_authoring_image(
                other.id,
                other.revision,
                asset.asset_id,
                ImagePlacement {
                    page: 0,
                    placement: [0.0, 0.0, 80.0, 60.0],
                    opacity: 1.0
                }
            )
            .is_err()
    );
    workspace.activate(first.id).unwrap();
    workspace
        .insert_authoring_image(
            first.id,
            first.revision,
            asset.asset_id,
            ImagePlacement {
                page: 0,
                placement: [0.0, 0.0, 80.0, 60.0],
                opacity: 1.0,
            },
        )
        .unwrap();
    let info = workspace.document(first.id).unwrap().info().unwrap();
    workspace
        .ensure_export_ready(info.id, info.revision)
        .unwrap();
    let stale = workspace
        .stage_authoring_image(info.id, info.revision, &png())
        .unwrap();
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert!(
        workspace
            .insert_authoring_image(
                info.id,
                0,
                stale.asset_id,
                ImagePlacement {
                    page: 0,
                    placement: [0.0, 0.0, 80.0, 60.0],
                    opacity: 1.0
                }
            )
            .is_err()
    );
    workspace
        .cancel_authoring_image(info.id, stale.asset_id)
        .unwrap();
}

#[test]
fn watermarks_and_numbers_embed_fonts_are_replaceable_and_follow_selected_range_order() {
    let mut workspace = workspace();
    apply(
        &mut workspace,
        Edit::Watermark {
            pages: vec![0, 1, 2, 3],
            settings: watermark("Résumé Ελληνικά Привет"),
        },
    );
    apply(
        &mut workspace,
        Edit::PageNumbers {
            pages: vec![2, 0, 3],
            settings: numbers(),
        },
    );
    let info = workspace.current().unwrap().info().unwrap();
    let labels: Vec<_> = info
        .annotations
        .iter()
        .filter(|annotation| annotation.kind == "page_number")
        .map(|annotation| (annotation.page, annotation.text.as_str()))
        .collect();
    assert_eq!(
        labels,
        [
            (0, "Page 8 / review"),
            (2, "Page 7 / review"),
            (3, "Page 9 / review")
        ]
    );
    let pdf = pdf(&workspace);
    for page in pdf.get_pages().into_values() {
        for annotation in pdf.get_page_annotations(page).unwrap() {
            let form = appearance(&pdf, annotation);
            let font = form
                .dict
                .get(b"Resources")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"Font")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"FolioFont")
                .unwrap();
            let font = pdf.dereference(font).unwrap().1.as_dict().unwrap();
            assert_eq!(font.get(b"Subtype").unwrap().as_name().unwrap(), b"Type0");
            assert!(font.has(b"ToUnicode"));
            assert!(
                Content::decode(&form.get_plain_content().unwrap())
                    .unwrap()
                    .operations
                    .iter()
                    .any(|operation| operation.operator == "Tj")
            );
        }
    }
    apply(
        &mut workspace,
        Edit::Watermark {
            pages: vec![0, 2],
            settings: watermark("Updated"),
        },
    );
    let info = workspace.current().unwrap().info().unwrap();
    assert_eq!(
        info.annotations
            .iter()
            .filter(|annotation| annotation.kind == "watermark")
            .count(),
        4
    );
    assert_eq!(
        info.annotations
            .iter()
            .filter(|annotation| annotation.text == "Updated")
            .count(),
        2
    );
    apply(
        &mut workspace,
        Edit::RemoveAuthoring {
            pages: vec![0, 2],
            kind: StampKind::Watermark,
        },
    );
    let bytes = workspace.current().unwrap().bytes().to_vec();
    let mut reopened = Session::from_bytes(bytes, "Stamps.pdf".into(), None, 2).unwrap();
    assert_eq!(reopened.info().unwrap().annotations.len(), 5);
    let before = reopened.bytes().to_vec();
    for edit in [
        Edit::Watermark {
            pages: vec![0, 1],
            settings: watermark("東京"),
        },
        Edit::Watermark {
            pages: vec![0],
            settings: watermark(&"Too long ".repeat(100)),
        },
        Edit::Watermark {
            pages: vec![0],
            settings: WatermarkSettings {
                opacity: 0.0,
                ..watermark("bad")
            },
        },
        Edit::Watermark {
            pages: vec![3, 0],
            settings: WatermarkSettings {
                size: 48.0,
                angle: 0.0,
                ..watermark("Wide watermark test")
            },
        },
        Edit::PageNumbers {
            pages: vec![0, 1],
            settings: PageNumberSettings {
                start: 999_999,
                ..numbers()
            },
        },
        Edit::PageNumbers {
            pages: vec![0],
            settings: PageNumberSettings {
                margin: f32::INFINITY,
                ..numbers()
            },
        },
        Edit::PageNumbers {
            pages: vec![3, 0],
            settings: PageNumberSettings {
                margin: 300.0,
                ..numbers()
            },
        },
    ] {
        assert!(reopened.apply(edit).is_err());
        assert_eq!(reopened.bytes(), before);
    }
}

#[test]
fn cropped_pages_keep_writing_source_text_and_redaction_export_geometry_consistent() {
    let mut workspace = workspace();
    apply(
        &mut workspace,
        Edit::Crop {
            pages: vec![0, 1, 2, 3],
            margins: [10.0, 20.0, 30.0, 40.0],
        },
    );
    apply(
        &mut workspace,
        Edit::Text {
            page: 0,
            rect: [80.0, 250.0, 250.0, 370.0],
            text: "Placed on cropped page".into(),
            size: 12.0,
            color: [0.0, 0.0, 0.0],
        },
    );
    apply(
        &mut workspace,
        Edit::Ink {
            page: 1,
            points: vec![[80.0, 200.0], [120.0, 220.0]],
            color: [0.0, 0.0, 1.0],
            width: 2.0,
        },
    );
    let runs = workspace.current().unwrap().text_runs(0).unwrap();
    assert_eq!(runs[0].origin, Some([80.0, 200.0]));
    apply(
        &mut workspace,
        Edit::ReplaceText {
            page: 0,
            path: runs[0].path.clone(),
            expected: "Original page 1".into(),
            replacement: "Edited cropped page".into(),
            font: ReplacementFont::Original,
        },
    );
    let info = workspace.current().unwrap().info().unwrap();
    let plan = workspace
        .begin_redaction(
            info.id,
            info.revision,
            150,
            &[Region {
                page: 0,
                rect: [80.0, 180.0, 120.0, 210.0],
            }],
        )
        .unwrap();
    for (page, raster) in plan.pages.iter().enumerate() {
        assert_eq!(
            raster.source_bounds,
            info.pages[page].geometry.bounds.map(f64::from)
        );
        assert_eq!(
            raster.width_points,
            f64::from(info.pages[page].geometry.width_points)
        );
        assert_eq!(
            raster.height_points,
            f64::from(info.pages[page].geometry.height_points)
        );
        let mut bytes = Vec::new();
        {
            let mut encoder =
                png::Encoder::new(&mut bytes, raster.pixel_width, raster.pixel_height);
            encoder.set_color(png::ColorType::Rgb);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![
                    255;
                    raster.pixel_width as usize
                        * raster.pixel_height as usize
                        * 3
                ])
                .unwrap();
        }
        workspace
            .stage_redaction_page(plan.job_id, page, &bytes)
            .unwrap();
    }
    let exported = workspace.redaction_bytes(plan.job_id).unwrap();
    let output = Document::load_mem(&exported).unwrap();
    assert_eq!(output.get_pages().len(), 4);
    assert!(
        output
            .extract_text(&[1, 2, 3, 4])
            .unwrap()
            .trim()
            .is_empty()
    );
    assert!(
        output
            .get_page_annotations(output.get_pages()[&1])
            .unwrap()
            .is_empty()
    );
    assert!(
        workspace.current().unwrap().text_runs(0).unwrap()[0]
            .text
            .as_ref()
            .unwrap()
            .contains("Edited cropped")
    );
}

#[test]
fn authoring_history_and_encrypted_disk_copies_keep_protection_and_original_files() {
    let mut document = Document::load_mem(&source()).unwrap();
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal("synthetic-authoring-id"),
            Object::string_literal("synthetic-authoring-id"),
        ],
    );
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner",
        user_password: "reader",
        permissions: Permissions::all(),
        key_length: 128,
    })
    .unwrap();
    document.encrypt(&state).unwrap();
    let mut original = Vec::new();
    document.save_to(&mut original).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let source_path = folder.path().join("original.pdf");
    std::fs::write(&source_path, &original).unwrap();
    let mut workspace = Workspace::default();
    let challenge = workspace.request_open_path(source_path.clone()).unwrap();
    let OpenOutcome::PasswordRequired { request_id, .. } = challenge else {
        panic!("expected password challenge")
    };
    workspace.unlock_document(request_id, "reader").unwrap();
    let info = workspace.current().unwrap().info().unwrap();
    let asset = workspace
        .stage_authoring_image(info.id, info.revision, &png())
        .unwrap();
    workspace
        .insert_authoring_image(
            info.id,
            info.revision,
            asset.asset_id,
            ImagePlacement {
                page: 0,
                placement: [20.0, 20.0, 80.0, 60.0],
                opacity: 0.8,
            },
        )
        .unwrap();
    apply(
        &mut workspace,
        Edit::Crop {
            pages: vec![0],
            margins: [10.0; 4],
        },
    );
    apply(
        &mut workspace,
        Edit::Watermark {
            pages: vec![1],
            settings: watermark("Protected"),
        },
    );
    apply(
        &mut workspace,
        Edit::PageNumbers {
            pages: vec![0, 1, 2, 3],
            settings: numbers(),
        },
    );
    let final_info = workspace.current().unwrap().info().unwrap();
    for _ in 0..4 {
        let info = workspace.current().unwrap().info().unwrap();
        workspace
            .current_mut(info.id, info.revision)
            .unwrap()
            .undo()
            .unwrap();
    }
    assert_eq!(workspace.current().unwrap().bytes(), original);
    for _ in 0..4 {
        let info = workspace.current().unwrap().info().unwrap();
        workspace
            .current_mut(info.id, info.revision)
            .unwrap()
            .redo()
            .unwrap();
    }
    assert_eq!(
        workspace.current().unwrap().info().unwrap().revision,
        final_info.revision
    );
    let path = folder.path().join("authored.pdf");
    workspace
        .current_mut(final_info.id, final_info.revision)
        .unwrap()
        .save_as(&path)
        .unwrap();
    assert_eq!(std::fs::read(source_path).unwrap(), original);
    let bytes = std::fs::read(path).unwrap();
    assert!(Document::load_mem(&bytes).unwrap().is_encrypted());
    for password in ["reader", "owner"] {
        let reopened = Session::from_bytes_with_password(
            bytes.clone(),
            "Authored.pdf".into(),
            None,
            99,
            Some(password),
        )
        .unwrap();
        assert_eq!(reopened.info().unwrap().annotations.len(), 6);
        assert!(reopened.info().unwrap().pages[0].geometry.crop_modified);
        let decrypted =
            Document::load_mem_with_options(&bytes, LoadOptions::with_password(password)).unwrap();
        assert_eq!(
            decrypted.encryption_state.unwrap().file_encryption_key(),
            state.file_encryption_key()
        );
    }
}

#[test]
fn signed_restricted_form_and_xfa_guards_remain_in_place() {
    for kind in ["signed", "form", "xfa", "restricted"] {
        let mut source = Document::load_mem(&source()).unwrap();
        if kind == "signed" {
            source.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into()], "Contents" => Object::string_literal("synthetic marker") });
        } else if kind == "form" || kind == "xfa" {
            let field = source.add_object(dictionary! { "FT" => "Tx" });
            let mut form = dictionary! { "Fields" => vec![Object::Reference(field)] };
            if kind == "xfa" {
                form.set("XFA", Object::string_literal("synthetic XFA"));
            }
            source.catalog_mut().unwrap().set("AcroForm", form);
        } else {
            source.trailer.set(
                "ID",
                vec![
                    Object::string_literal("synthetic"),
                    Object::string_literal("synthetic"),
                ],
            );
            let state = EncryptionState::try_from(EncryptionVersion::V2 {
                document: &source,
                owner_password: "owner",
                user_password: "reader",
                permissions: Permissions::PRINTABLE,
                key_length: 128,
            })
            .unwrap();
            source.encrypt(&state).unwrap();
        }
        let mut bytes = Vec::new();
        source.save_to(&mut bytes).unwrap();
        let mut session = Session::from_bytes_with_password(
            bytes.clone(),
            "Guarded.pdf".into(),
            None,
            1,
            (kind == "restricted").then_some("reader"),
        )
        .unwrap();
        assert!(
            session
                .apply(Edit::Crop {
                    pages: vec![0],
                    margins: [1.0; 4]
                })
                .is_err(),
            "{kind}"
        );
        if kind != "form" {
            assert!(
                session
                    .apply(Edit::Watermark {
                        pages: vec![0],
                        settings: watermark("Blocked")
                    })
                    .is_err(),
                "{kind}"
            );
        }
        assert_eq!(session.bytes(), bytes);
    }
}

#[test]
fn transparent_appearances_upgrade_old_pdf_versions_and_undo_restores_original_bytes() {
    let mut source = Document::load_mem(&source()).unwrap();
    source.version = "1.3".into();
    let mut bytes = Vec::new();
    source.save_to(&mut bytes).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(bytes.clone(), "Old header.pdf".into())
        .unwrap();
    let asset = workspace
        .stage_authoring_image(info.id, info.revision, &png())
        .unwrap();
    workspace
        .insert_authoring_image(
            info.id,
            info.revision,
            asset.asset_id,
            ImagePlacement {
                page: 0,
                placement: [20.0, 20.0, 80.0, 60.0],
                opacity: 0.5,
            },
        )
        .unwrap();
    assert_eq!(pdf(&workspace).version, "1.4");
    let info = workspace.current().unwrap().info().unwrap();
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert_eq!(workspace.current().unwrap().bytes(), bytes);
    apply(
        &mut workspace,
        Edit::Watermark {
            pages: vec![0],
            settings: watermark("Review"),
        },
    );
    assert_eq!(pdf(&workspace).version, "1.4");
}

#[test]
fn image_soft_masks_embedded_stamp_fonts_and_crops_keep_aes_128_and_256_protection() {
    for version in [4, 5] {
        let mut document = Document::load_mem(&source()).unwrap();
        document.trailer.set(
            "ID",
            vec![
                Object::string_literal("aes-authoring"),
                Object::string_literal("aes-authoring"),
            ],
        );
        let filter: Arc<dyn CryptFilter> = if version == 4 {
            Arc::new(Aes128CryptFilter)
        } else {
            Arc::new(Aes256CryptFilter)
        };
        let filters = BTreeMap::from([(b"StdCF".to_vec(), filter)]);
        let method = if version == 4 {
            EncryptionVersion::V4 {
                document: &document,
                owner_password: "owner",
                user_password: "reader",
                permissions: Permissions::all(),
                encrypt_metadata: true,
                crypt_filters: filters,
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        } else {
            EncryptionVersion::V5 {
                owner_password: "owner",
                user_password: "reader",
                permissions: Permissions::all(),
                encrypt_metadata: true,
                file_encryption_key: &[57; 32],
                crypt_filters: filters,
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        };
        let state = EncryptionState::try_from(method).unwrap();
        document.encrypt(&state).unwrap();
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        let mut workspace = Workspace::default();
        let OpenOutcome::PasswordRequired { request_id, .. } = workspace
            .request_open_bytes(bytes, "AES.pdf".into())
            .unwrap()
        else {
            panic!("expected password challenge")
        };
        workspace.unlock_document(request_id, "reader").unwrap();
        let info = workspace.current().unwrap().info().unwrap();
        let asset = workspace
            .stage_authoring_image(info.id, info.revision, &png())
            .unwrap();
        workspace
            .insert_authoring_image(
                info.id,
                info.revision,
                asset.asset_id,
                ImagePlacement {
                    page: 0,
                    placement: [20.0, 20.0, 80.0, 60.0],
                    opacity: 0.5,
                },
            )
            .unwrap();
        apply(
            &mut workspace,
            Edit::Crop {
                pages: vec![0],
                margins: [2.0; 4],
            },
        );
        apply(
            &mut workspace,
            Edit::Watermark {
                pages: vec![0],
                settings: watermark("Protected"),
            },
        );
        apply(
            &mut workspace,
            Edit::PageNumbers {
                pages: vec![0],
                settings: numbers(),
            },
        );
        let bytes = workspace.current().unwrap().bytes().to_vec();
        assert!(Document::load_mem(&bytes).unwrap().is_encrypted());
        for password in ["reader", "owner"] {
            let reopened = Session::from_bytes_with_password(
                bytes.clone(),
                "AES copy.pdf".into(),
                None,
                99,
                Some(password),
            )
            .unwrap();
            assert_eq!(reopened.info().unwrap().annotations.len(), 3);
            assert!(reopened.info().unwrap().pages[0].geometry.crop_modified);
            let document = Document::load_mem(&reopened.render_bytes().unwrap()).unwrap();
            let images: Vec<_> = document
                .objects
                .values()
                .filter_map(|object| object.as_stream().ok())
                .filter(|stream| {
                    stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
                })
                .collect();
            assert_eq!(images.len(), 2);
            assert!(images.iter().any(|stream| stream.dict.has(b"SMask")));
        }
    }
}

#[test]
fn write_original_synthetic_authoring_fixtures_when_requested() {
    if let Ok(folder) = std::env::var("FOLIO_AUTHORING_FIXTURES") {
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("authoring-original.pdf"), source()).unwrap();
        std::fs::write(folder.join("authoring-alpha.png"), png()).unwrap();
        std::fs::write(folder.join("authoring-oriented.jpg"), oriented_jpeg()).unwrap();
        let mut workspace = workspace();
        for page in 0..4 {
            let info = workspace.current().unwrap().info().unwrap();
            let asset = workspace
                .stage_authoring_image(info.id, info.revision, &png())
                .unwrap();
            workspace
                .insert_authoring_image(
                    info.id,
                    info.revision,
                    asset.asset_id,
                    ImagePlacement {
                        page,
                        placement: [20.0, 40.0, 80.0, 60.0],
                        opacity: 0.5,
                    },
                )
                .unwrap();
        }
        apply(
            &mut workspace,
            Edit::Watermark {
                pages: vec![0, 1, 2, 3],
                settings: watermark("Folio review"),
            },
        );
        apply(
            &mut workspace,
            Edit::PageNumbers {
                pages: vec![0, 1, 2, 3],
                settings: numbers(),
            },
        );
        std::fs::write(
            folder.join("authoring-stamped.pdf"),
            workspace.current().unwrap().bytes(),
        )
        .unwrap();
        apply(
            &mut workspace,
            Edit::Crop {
                pages: vec![0, 1, 2, 3],
                margins: [10.0, 20.0, 30.0, 40.0],
            },
        );
        std::fs::write(
            folder.join("authoring-cropped.pdf"),
            workspace.current().unwrap().bytes(),
        )
        .unwrap();
        apply(
            &mut workspace,
            Edit::ResetCrop {
                pages: vec![0, 1, 2, 3],
            },
        );
        std::fs::write(
            folder.join("authoring-reset.pdf"),
            workspace.current().unwrap().bytes(),
        )
        .unwrap();
    }
}
