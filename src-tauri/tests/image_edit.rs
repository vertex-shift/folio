use std::io::Cursor;

use folio_core::{
    document::{DocumentInfo, Edit, Session},
    image_edit::{ImageChange, ImageOccurrence},
    workspace::{OpenOutcome, Workspace},
};
use image::{DynamicImage, ImageFormat, RgbImage};
use lopdf::{
    Dictionary, Document, LoadOptions, Object, ObjectId, Stream,
    content::Content,
    dictionary,
    encryption::{EncryptionState, EncryptionVersion, Permissions},
};

fn numbers(values: &[f32]) -> Object {
    Object::Array(values.iter().map(|value| Object::Real(*value)).collect())
}

fn image(pdf: &mut Document) -> ObjectId {
    pdf.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 1,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
        },
        vec![220, 30, 40, 20, 60, 220],
    ))
}

fn bytes(mut pdf: Document) -> Vec<u8> {
    let mut result = Vec::new();
    pdf.save_to(&mut result).unwrap();
    result
}

fn direct(content: &[u8]) -> Document {
    let mut pdf = Document::with_version("1.7");
    let root = pdf.new_object_id();
    let image = image(&mut pdf);
    let stream = pdf.add_object(Stream::new(dictionary! {}, content.to_vec()));
    let page = pdf.add_object(dictionary! {
        "Type" => "Page", "Parent" => root, "Contents" => stream,
    });
    pdf.objects.insert(
        root,
        dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
            "MediaBox" => numbers(&[-20.0, -30.0, 612.0, 792.0]),
            "CropBox" => numbers(&[0.0, 10.0, 600.0, 780.0]),
            "Resources" => dictionary! { "XObject" => dictionary! { "Img" => image } },
        }
        .into(),
    );
    let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
    pdf.trailer.set("Root", catalog);
    pdf
}

fn shared_nested() -> Document {
    let mut pdf = direct(b"");
    let first = pdf.get_pages()[&1];
    let root = pdf
        .get_dictionary(first)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let resources = pdf
        .get_dictionary(root)
        .unwrap()
        .get(b"Resources")
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let inner = pdf.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[0.0, 0.0, 210.0, 100.0]),
            "Resources" => resources,
        },
        b"q 80 0 0 60 10 20 cm /Img Do Q q 80 0 0 60 110 20 cm /Img Do Q".to_vec(),
    ));
    let outer = pdf.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[0.0, 0.0, 230.0, 140.0]),
            "Matrix" => numbers(&[1.0, 0.0, 0.0, 1.0, 10.0, 20.0]),
            "Resources" => dictionary! { "XObject" => dictionary! { "Inner" => inner } },
        },
        b"/Inner Do".to_vec(),
    ));
    let font = pdf.add_object(
        dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" },
    );
    let resources = pdf.add_object(dictionary! {
        "XObject" => dictionary! { "Outer" => outer },
        "Font" => dictionary! { "F1" => font },
    });
    pdf.get_dictionary_mut(root)
        .unwrap()
        .set("Resources", resources);
    let stream = pdf.add_object(Stream::new(dictionary! {},
        b"0.2 0.6 0.4 rg 30 350 50 30 re f BT /F1 12 Tf 40 700 Td (Untouched source text) Tj ET q 1 0 0 1 40 100 cm /Outer Do Q q 1 0 0 1 320 100 cm /Outer Do Q".to_vec()));
    pdf.get_dictionary_mut(first)
        .unwrap()
        .set("Contents", vec![Object::Reference(stream)]);
    let second =
        pdf.add_object(dictionary! { "Type" => "Page", "Parent" => root, "Contents" => stream });
    pdf.get_dictionary_mut(root).unwrap().set(
        "Kids",
        vec![Object::Reference(first), Object::Reference(second)],
    );
    pdf.get_dictionary_mut(root).unwrap().set("Count", 2);
    let annotation_image = image(&mut pdf);
    let appearance = pdf.add_object(Stream::new(dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[0.0, 0.0, 20.0, 20.0]),
        "Resources" => dictionary! { "XObject" => dictionary! { "NotPageContent" => annotation_image } },
    }, b"20 0 0 20 0 0 cm /NotPageContent Do".to_vec()));
    let annotation = pdf.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Stamp", "Rect" => numbers(&[30.0, 30.0, 50.0, 50.0]),
        "AP" => dictionary! { "N" => appearance }, "P" => first,
    });
    pdf.get_dictionary_mut(first)
        .unwrap()
        .set("Annots", vec![Object::Reference(annotation)]);
    pdf
}

fn open_workspace(pdf: Document) -> (Workspace, DocumentInfo) {
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(bytes(pdf), "Synthetic image occurrences.pdf".into())
        .unwrap();
    (workspace, info)
}

fn occurrences(workspace: &Workspace, id: u64, page: usize) -> Vec<ImageOccurrence> {
    workspace
        .document(id)
        .unwrap()
        .image_occurrences(page)
        .unwrap()
        .images
}

fn current(workspace: &Workspace, id: u64) -> DocumentInfo {
    workspace.document(id).unwrap().info().unwrap()
}

fn apply(workspace: &mut Workspace, id: u64, change: ImageChange) -> DocumentInfo {
    workspace
        .edit_image_occurrence(id, current(workspace, id).revision, change)
        .unwrap()
}

fn rgba_png() -> Vec<u8> {
    let mut result = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut result, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("Comment".into(), "PRIVATE_SYNTHETIC_IMAGE_METADATA".into())
            .unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[0, 240, 60, 128, 255, 90, 0, 0])
            .unwrap();
        writer.finish().unwrap();
    }
    result
}

fn oriented_jpeg() -> Vec<u8> {
    let image = DynamicImage::ImageRgb8(RgbImage::from_fn(3, 2, |x, y| {
        image::Rgb([x as u8 * 70, y as u8 * 90, 80])
    }));
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, ImageFormat::Jpeg).unwrap();
    let encoded = encoded.into_inner();
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut result = encoded[..2].to_vec();
    result.extend_from_slice(&[0xff, 0xe1]);
    result.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    result.extend_from_slice(exif);
    result.extend_from_slice(&encoded[2..]);
    result
}

fn page_resources(pdf: &Document, id: ObjectId) -> Dictionary {
    let page = pdf.get_dictionary(id).unwrap();
    if let Ok(value) = page.get(b"Resources") {
        pdf.dereference(value).unwrap().1.as_dict().unwrap().clone()
    } else {
        page_resources(pdf, page.get(b"Parent").unwrap().as_reference().unwrap())
    }
}

fn stream_for_path(pdf: &Document, page: usize, path: &[usize]) -> Stream {
    let id = pdf.get_pages()[&(page as u32 + 1)];
    let mut content = pdf.get_page_content(id);
    let mut resources = page_resources(pdf, id);
    for (depth, index) in path.iter().enumerate() {
        let decoded = Content::decode(&content).unwrap();
        let op = &decoded.operations[*index];
        assert_eq!(op.operator, "Do");
        let xobjects = pdf
            .dereference(resources.get(b"XObject").unwrap())
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        let stream = pdf
            .dereference(xobjects.get(op.operands[0].as_name().unwrap()).unwrap())
            .unwrap()
            .1
            .as_stream()
            .unwrap();
        if depth == path.len() - 1 {
            return stream.clone();
        }
        content = stream
            .get_plain_content_with_limit(16 * 1024 * 1024)
            .unwrap();
        if let Ok(value) = stream.dict.get(b"Resources") {
            resources = pdf.dereference(value).unwrap().1.as_dict().unwrap().clone();
        }
    }
    panic!("no image at path")
}

fn assert_placement(actual: Option<[f32; 4]>, expected: [f32; 4]) {
    for (a, b) in actual.unwrap().into_iter().zip(expected) {
        assert!((a - b).abs() < 0.002, "{a} != {b}");
    }
}

#[test]
fn real_image_backed_checkbox_appearances_are_not_page_content_targets_and_survive_edits() {
    let mut source = shared_nested();
    let page = source.get_pages()[&1];
    let stamp = source
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_reference()
        .unwrap();
    let appearance = source
        .get_dictionary(stamp)
        .unwrap()
        .get(b"AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"N")
        .unwrap()
        .clone();
    let widget = source.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn",
        "T" => Object::string_literal("Synthetic image checkbox"), "F" => 4,
        "Rect" => numbers(&[80.0, 30.0, 100.0, 50.0]), "P" => page,
        "V" => "Off", "AS" => "Off",
        "AP" => dictionary! { "N" => dictionary! { "Off" => appearance.clone(), "Yes" => appearance.clone() } },
    });
    source
        .get_dictionary_mut(page)
        .unwrap()
        .get_mut(b"Annots")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(Object::Reference(widget));
    source.catalog_mut().unwrap().set(
        "AcroForm",
        dictionary! { "Fields" => vec![Object::Reference(widget)] },
    );
    let (mut workspace, info) = open_workspace(source);
    let original = Document::load_mem(workspace.document(info.id).unwrap().bytes()).unwrap();
    let before_widget = original.get_object(widget).unwrap().clone();
    let before_appearance = original.dereference(&appearance).unwrap().1.clone();
    let images = occurrences(&workspace, info.id, 0);
    assert_eq!(images.len(), 4);
    assert!(images.iter().all(|image| image.editable));
    apply(
        &mut workspace,
        info.id,
        ImageChange::Remove {
            page: 0,
            path: images[0].path.clone(),
        },
    );
    let saved = workspace.document(info.id).unwrap().bytes().to_vec();
    let reopened =
        Session::from_bytes(saved.clone(), "Image widget.pdf".into(), None, 100).unwrap();
    assert_eq!(reopened.image_occurrences(0).unwrap().images.len(), 3);
    assert_eq!(reopened.image_occurrences(1).unwrap().images.len(), 4);
    assert_eq!(reopened.form_fields().unwrap().fields.len(), 1);
    let pdf = Document::load_mem(&saved).unwrap();
    assert_eq!(pdf.get_object(widget).unwrap(), &before_widget);
    assert_eq!(pdf.dereference(&appearance).unwrap().1, &before_appearance);
}

#[test]
fn replaces_one_nested_repeated_occurrence_without_touching_shared_pages_or_annotation_images() {
    let (mut workspace, info) = open_workspace(shared_nested());
    let original = workspace.document(info.id).unwrap().bytes().to_vec();
    let before_pdf = Document::load_mem(&original).unwrap();
    let before = occurrences(&workspace, info.id, 0);
    assert_eq!(before.len(), 4);
    assert!(
        before
            .iter()
            .all(|image| image.editable && image.path.len() == 3)
    );
    assert_eq!(before[0].placement, Some([60.0, 580.0, 80.0, 60.0]));
    let asset = workspace
        .stage_image_replacement(info.id, info.revision, &rgba_png())
        .unwrap();
    let changed = apply(
        &mut workspace,
        info.id,
        ImageChange::Replace {
            page: 0,
            path: before[1].path.clone(),
            asset_id: asset.asset_id,
            placement: None,
        },
    );
    let after_pdf = Document::load_mem(workspace.document(info.id).unwrap().bytes()).unwrap();
    let after = occurrences(&workspace, info.id, 0);
    for (index, occurrence) in after.iter().enumerate() {
        assert_eq!(occurrence.placement, before[index].placement);
        let source = stream_for_path(&after_pdf, 0, &occurrence.path);
        assert_eq!(
            source.get_plain_content_with_limit(100).unwrap(),
            if index == 1 {
                vec![0, 240, 60, 0, 0, 0]
            } else {
                vec![220, 30, 40, 20, 60, 220]
            }
        );
        assert_eq!(source.dict.has(b"SMask"), index == 1);
    }
    assert_eq!(
        occurrences(&workspace, info.id, 1)
            .iter()
            .map(|image| &image.path)
            .collect::<Vec<_>>(),
        before.iter().map(|image| &image.path).collect::<Vec<_>>()
    );
    assert_eq!(
        after_pdf.get_page_content(after_pdf.get_pages()[&2]),
        before_pdf.get_page_content(before_pdf.get_pages()[&2])
    );
    for (id, object) in &before_pdf.objects {
        if *id == before_pdf.get_pages()[&1]
            || object.as_stream().ok().is_some_and(|stream| {
                stream.dict.get(b"Type").and_then(Object::as_name).ok() == Some(b"XRef")
            })
        {
            continue;
        }
        assert_eq!(
            after_pdf.objects.get(id),
            Some(object),
            "original object {id:?} changed"
        );
    }
    assert_eq!(
        after_pdf.extract_text(&[1, 2]).unwrap(),
        before_pdf.extract_text(&[1, 2]).unwrap()
    );
    assert_eq!(changed.annotations.len(), info.annotations.len());
    assert!(
        !String::from_utf8_lossy(workspace.document(info.id).unwrap().bytes())
            .contains("PRIVATE_SYNTHETIC_IMAGE_METADATA")
    );
    workspace
        .current_mut(info.id, changed.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert_eq!(workspace.document(info.id).unwrap().bytes(), original);
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .redo()
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("replaced.pdf");
    workspace
        .current_mut(info.id, changed.revision)
        .unwrap()
        .save_as(&path)
        .unwrap();
    let reopened = Session::from_path(path, 99).unwrap();
    assert_eq!(reopened.image_occurrences(0).unwrap().images.len(), 4);
    assert_eq!(reopened.image_occurrences(1).unwrap().images.len(), 4);
    let reopened_pdf = Document::load_mem(reopened.bytes()).unwrap();
    assert!(
        stream_for_path(
            &reopened_pdf,
            0,
            &reopened.image_occurrences(0).unwrap().images[1].path
        )
        .dict
        .has(b"SMask")
    );
    assert!(
        !stream_for_path(&reopened_pdf, 1, &before[1].path)
            .dict
            .has(b"SMask")
    );
}

#[test]
fn removes_and_moves_only_the_selected_nested_invocation_and_keeps_form_clipping() {
    let (mut workspace, info) = open_workspace(shared_nested());
    let before = occurrences(&workspace, info.id, 0);
    apply(
        &mut workspace,
        info.id,
        ImageChange::Remove {
            page: 0,
            path: before[1].path.clone(),
        },
    );
    let remaining = occurrences(&workspace, info.id, 0);
    assert_eq!(remaining.len(), 3);
    assert_eq!(
        remaining
            .iter()
            .map(|image| image.placement)
            .collect::<Vec<_>>(),
        vec![
            before[0].placement,
            before[2].placement,
            before[3].placement
        ]
    );
    assert_eq!(occurrences(&workspace, info.id, 1).len(), 4);
    let moved = apply(
        &mut workspace,
        info.id,
        ImageChange::Place {
            page: 0,
            path: remaining[0].path.clone(),
            placement: [70.0, 570.0, 60.0, 40.0],
        },
    );
    assert_placement(
        occurrences(&workspace, info.id, 0)[0].placement,
        [70.0, 570.0, 60.0, 40.0],
    );
    let before = workspace.document(info.id).unwrap().bytes().to_vec();
    let occurrence = occurrences(&workspace, info.id, 0).remove(0);
    assert!(
        workspace
            .edit_image_occurrence(
                info.id,
                moved.revision,
                ImageChange::Place {
                    page: 0,
                    path: occurrence.path,
                    placement: [10.0, 10.0, 60.0, 40.0]
                }
            )
            .unwrap_err()
            .to_string()
            .contains("Form BBox")
    );
    assert_eq!(workspace.document(info.id).unwrap().bytes(), before);
    let reopened = Session::from_bytes(before, "moved.pdf".into(), None, 90).unwrap();
    assert_placement(
        reopened.image_occurrences(0).unwrap().images[0].placement,
        [70.0, 570.0, 60.0, 40.0],
    );
    assert_eq!(reopened.image_occurrences(1).unwrap().images.len(), 4);
}

#[test]
fn geometry_edits_use_crop_rotation_and_page_local_user_unit_and_preserve_reflections() {
    for rotation in [0, 90, 180, 270] {
        for transform in [
            "80 0 0 60 80 140",
            "-80 0 0 60 160 140",
            "0 80 -60 0 160 140",
        ] {
            let mut source = direct(format!("q {transform} cm /Img Do Q").as_bytes());
            let page = source.get_pages()[&1];
            let parent = source
                .get_dictionary(page)
                .unwrap()
                .get(b"Parent")
                .unwrap()
                .as_reference()
                .unwrap();
            source
                .get_dictionary_mut(parent)
                .unwrap()
                .set("UserUnit", 100.0_f32);
            source
                .get_dictionary_mut(parent)
                .unwrap()
                .set("Rotate", rotation);
            source
                .get_dictionary_mut(page)
                .unwrap()
                .set("UserUnit", 2.5_f32);
            let (mut workspace, info) = open_workspace(source);
            assert_eq!(info.pages[0].geometry.user_unit, 2.5);
            let original = occurrences(&workspace, info.id, 0).remove(0);
            assert!(original.editable, "{:?}", original.reason);
            apply(
                &mut workspace,
                info.id,
                ImageChange::Place {
                    page: 0,
                    path: original.path,
                    placement: [40.0, 60.0, 125.0, 75.0],
                },
            );
            assert_placement(
                occurrences(&workspace, info.id, 0)[0].placement,
                [40.0, 60.0, 125.0, 75.0],
            );
            let reopened = Session::from_bytes(
                workspace.document(info.id).unwrap().bytes().to_vec(),
                "geometry.pdf".into(),
                None,
                20,
            )
            .unwrap();
            assert_placement(
                reopened.image_occurrences(0).unwrap().images[0].placement,
                [40.0, 60.0, 125.0, 75.0],
            );
        }
    }
}

#[test]
fn extreme_integer_origins_are_indexed_exactly_but_precision_losing_placements_are_atomic_errors() {
    let mut source = direct(b"q 80 0 0 64 100000081 100000144 cm /Img Do Q");
    let page = source.get_pages()[&1];
    source.get_dictionary_mut(page).unwrap().set(
        "MediaBox",
        numbers(&[100000000.0, 100000000.0, 100001024.0, 100001024.0]),
    );
    source.get_dictionary_mut(page).unwrap().set(
        "CropBox",
        numbers(&[100000000.0, 100000000.0, 100001024.0, 100001024.0]),
    );
    let (mut workspace, info) = open_workspace(source);
    let original = workspace.document(info.id).unwrap().bytes().to_vec();
    let image = occurrences(&workspace, info.id, 0).remove(0);
    assert_placement(image.placement, [81.0, 816.0, 80.0, 64.0]);
    assert!(image.editable);
    let error = workspace
        .edit_image_occurrence(
            info.id,
            info.revision,
            ImageChange::Place {
                page: 0,
                path: image.path,
                placement: [85.0, 816.0, 80.0, 64.0],
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("precision"));
    assert_eq!(workspace.document(info.id).unwrap().bytes(), original);
    assert!(!current(&workspace, info.id).can_undo);
}

#[test]
fn alpha_and_orientation_share_the_bounded_sanitized_authoring_decoder() {
    let (mut workspace, info) = open_workspace(direct(b"q 80 0 0 60 80 140 cm /Img Do Q"));
    let target = occurrences(&workspace, info.id, 0).remove(0);
    let asset = workspace
        .stage_image_replacement(info.id, info.revision, &oriented_jpeg())
        .unwrap();
    assert_eq!((asset.width, asset.height), (2, 3));
    apply(
        &mut workspace,
        info.id,
        ImageChange::Replace {
            page: 0,
            path: target.path,
            asset_id: asset.asset_id,
            placement: None,
        },
    );
    let current = current(&workspace, info.id);
    let occurrence = occurrences(&workspace, info.id, 0).remove(0);
    assert_eq!(
        (occurrence.pixel_width, occurrence.pixel_height),
        (Some(2), Some(3))
    );
    let asset = workspace
        .stage_image_replacement(info.id, current.revision, &rgba_png())
        .unwrap();
    apply(
        &mut workspace,
        info.id,
        ImageChange::Replace {
            page: 0,
            path: occurrence.path,
            asset_id: asset.asset_id,
            placement: Some([40.0, 50.0, 100.0, 80.0]),
        },
    );
    let pdf = Document::load_mem(workspace.document(info.id).unwrap().bytes()).unwrap();
    let occurrence = occurrences(&workspace, info.id, 0).remove(0);
    assert_placement(occurrence.placement, [40.0, 50.0, 100.0, 80.0]);
    let image = stream_for_path(&pdf, 0, &occurrence.path);
    let mask = pdf
        .dereference(image.dict.get(b"SMask").unwrap())
        .unwrap()
        .1
        .as_stream()
        .unwrap();
    assert_eq!(mask.get_plain_content_with_limit(100).unwrap(), [128, 0]);
    assert_eq!(
        image.get_plain_content_with_limit(100).unwrap(),
        [0, 240, 60, 0, 0, 0]
    );
    assert!(!image.dict.has(b"Metadata"));
}

#[test]
fn clipping_optional_content_transparency_stencils_and_oblique_transforms_are_honestly_read_only() {
    for (content, expected) in [
        (
            "q 0 0 100 100 re W n 80 0 0 60 80 140 cm /Img Do Q",
            "clipping",
        ),
        ("q 80 10 0 60 80 140 cm /Img Do Q", "axis-aligned"),
        ("q 80 0 0 60 580 140 cm /Img Do Q", "clipped"),
        (
            "/OC <<>> BDC q 80 0 0 60 80 140 cm /Img Do Q EMC",
            "optional-content",
        ),
        ("q /Alpha gs 80 0 0 60 80 140 cm /Img Do Q", "transparency"),
        (
            "BT 7 Tr (clip) Tj ET q 80 0 0 60 80 140 cm /Img Do Q",
            "text clipping",
        ),
    ] {
        let mut source = direct(content.as_bytes());
        let page = source.get_pages()[&1];
        let root = source
            .get_dictionary(page)
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap();
        source
            .get_dictionary_mut(root)
            .unwrap()
            .get_mut(b"Resources")
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "ExtGState",
                dictionary! { "Alpha" => dictionary! { "ca" => 0.5_f32 } },
            );
        let (mut workspace, info) = open_workspace(source);
        let original = workspace.document(info.id).unwrap().bytes().to_vec();
        let target = occurrences(&workspace, info.id, 0).remove(0);
        assert!(!target.editable);
        assert!(target.reason.unwrap().contains(expected), "{content}");
        assert!(
            workspace
                .edit_image_occurrence(
                    info.id,
                    info.revision,
                    ImageChange::Remove {
                        page: 0,
                        path: target.path
                    }
                )
                .is_err()
        );
        assert_eq!(workspace.document(info.id).unwrap().bytes(), original);
    }
    let mut source = direct(b"q 80 0 0 60 80 140 cm /Img Do Q");
    let image = source
        .objects
        .iter()
        .find_map(|(id, object)| {
            object
                .as_stream()
                .ok()
                .filter(|stream| {
                    stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
                })
                .map(|_| *id)
        })
        .unwrap();
    source
        .get_object_mut(image)
        .unwrap()
        .as_stream_mut()
        .unwrap()
        .dict
        .set("ImageMask", true);
    let (workspace, info) = open_workspace(source);
    assert!(
        occurrences(&workspace, info.id, 0)[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("Stencil")
    );
}

#[test]
fn rejects_malformed_inline_cyclic_and_excessive_content_without_any_mutation() {
    for content in [
        b"Q q 80 0 0 60 80 140 cm /Img Do Q".as_slice(),
        b"q 80 0 0 60 80 140 cm /Img Do".as_slice(),
        b"BI /W 1 /H 1 /BPC 8 /CS /RGB ID \xff\x00\x00 EI q 80 0 0 60 80 140 cm /Img Do Q"
            .as_slice(),
    ] {
        let (mut workspace, info) = open_workspace(direct(content));
        let before = workspace.document(info.id).unwrap().bytes().to_vec();
        match workspace.document(info.id).unwrap().image_occurrences(0) {
            Ok(page) => assert!(page.images.iter().all(|image| !image.editable)),
            Err(error) => assert!(!error.to_string().is_empty()),
        }
        assert!(
            workspace
                .edit_image_occurrence(
                    info.id,
                    info.revision,
                    ImageChange::Remove {
                        page: 0,
                        path: vec![2]
                    }
                )
                .is_err()
        );
        assert_eq!(workspace.document(info.id).unwrap().bytes(), before);
    }
    let mut source = shared_nested();
    let form = source
        .objects
        .iter()
        .find_map(|(id, object)| {
            object
                .as_stream()
                .ok()
                .filter(|stream| {
                    stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form")
                })
                .map(|_| *id)
        })
        .unwrap();
    let stream = source
        .get_object_mut(form)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    stream.dict.set(
        "Resources",
        dictionary! { "XObject" => dictionary! { "Cycle" => form } },
    );
    stream.set_plain_content(b"/Cycle Do".to_vec());
    let (workspace, info) = open_workspace(source);
    assert!(
        workspace
            .document(info.id)
            .unwrap()
            .image_occurrences(0)
            .unwrap_err()
            .to_string()
            .contains("cyclic")
    );
    let (workspace, info) = open_workspace(direct("q Q ".repeat(100_001).as_bytes()));
    assert!(
        workspace
            .document(info.id)
            .unwrap()
            .image_occurrences(0)
            .unwrap_err()
            .to_string()
            .contains("200,000")
    );
}

#[test]
fn failures_keep_staged_pixels_and_history_atomic_and_assets_do_not_revive_after_undo() {
    let (mut workspace, info) = open_workspace(shared_nested());
    let target = occurrences(&workspace, info.id, 0).remove(0);
    let asset = workspace
        .stage_image_replacement(info.id, info.revision, &rgba_png())
        .unwrap();
    let original = workspace.document(info.id).unwrap().bytes().to_vec();
    assert!(
        workspace
            .stage_image_replacement(info.id, info.revision, b"not a PNG")
            .is_err()
    );
    assert!(
        workspace
            .cancel_image_replacement(info.id, asset.asset_id + 1)
            .is_err()
    );
    for change in [
        ImageChange::Replace {
            page: 0,
            path: target.path.clone(),
            asset_id: asset.asset_id,
            placement: Some([0.0, 0.0, 40.0, 40.0]),
        },
        ImageChange::Place {
            page: 0,
            path: target.path.clone(),
            placement: target.placement.unwrap(),
        },
        ImageChange::Remove {
            page: 0,
            path: vec![usize::MAX],
        },
        ImageChange::Remove {
            page: 5,
            path: target.path.clone(),
        },
    ] {
        assert!(
            workspace
                .edit_image_occurrence(info.id, info.revision, change)
                .is_err()
        );
        assert_eq!(workspace.document(info.id).unwrap().bytes(), original);
        assert!(!current(&workspace, info.id).can_undo);
    }
    assert!(
        workspace
            .edit_image_occurrence(
                info.id,
                info.revision + 1,
                ImageChange::Remove {
                    page: 0,
                    path: target.path.clone()
                }
            )
            .is_err()
    );
    let other = workspace
        .open_bytes(
            bytes(direct(b"q 80 0 0 60 80 140 cm /Img Do Q")),
            "other.pdf".into(),
        )
        .unwrap();
    assert!(
        workspace
            .edit_image_occurrence(
                other.id,
                other.revision,
                ImageChange::Replace {
                    page: 0,
                    path: vec![2],
                    asset_id: asset.asset_id,
                    placement: None
                }
            )
            .is_err()
    );
    let changed = workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    workspace
        .current_mut(info.id, changed.revision)
        .unwrap()
        .undo()
        .unwrap();
    assert_eq!(current(&workspace, info.id).revision, info.revision);
    assert!(
        workspace
            .edit_image_occurrence(
                info.id,
                info.revision,
                ImageChange::Replace {
                    page: 0,
                    path: target.path.clone(),
                    asset_id: asset.asset_id,
                    placement: None
                }
            )
            .unwrap_err()
            .to_string()
            .contains("stale")
    );
    workspace
        .cancel_image_replacement(info.id, asset.asset_id)
        .unwrap();
    let asset = workspace
        .stage_image_replacement(info.id, info.revision, &rgba_png())
        .unwrap();
    apply(
        &mut workspace,
        info.id,
        ImageChange::Replace {
            page: 0,
            path: target.path,
            asset_id: asset.asset_id,
            placement: None,
        },
    );
}

#[test]
fn replacement_assets_and_geometry_drafts_guard_outputs_signing_ocr_comparison_and_close() {
    let (mut workspace, info) = open_workspace(direct(b"q 80 0 0 60 80 140 cm /Img Do Q"));
    let other = workspace
        .open_bytes(
            bytes(direct(b"q 80 0 0 60 80 140 cm /Img Do Q")),
            "other.pdf".into(),
        )
        .unwrap();
    let asset = workspace
        .stage_image_replacement(info.id, info.revision, &rgba_png())
        .unwrap();
    assert!(workspace.has_unsaved_changes());
    assert!(
        workspace
            .ensure_export_ready(info.id, info.revision)
            .is_err()
    );
    assert!(workspace.check_signing(info.id, info.revision).is_err());
    assert!(
        workspace
            .close_document(info.id, info.revision, false)
            .is_err()
    );
    assert!(
        workspace
            .ensure_export_ready(other.id, other.revision)
            .is_ok()
    );
    assert!(
        workspace
            .begin_ocr(
                info.id,
                info.revision,
                folio_core::ocr::Options {
                    pages: vec![0],
                    dpi: 150,
                    language: "eng".into(),
                    existing_text: folio_core::ocr::ExistingTextPolicy::Force
                }
            )
            .is_err()
    );
    assert!(
        workspace
            .begin_redaction(
                info.id,
                info.revision,
                150,
                &[folio_core::redaction::Region {
                    page: 0,
                    rect: [80.0, 140.0, 100.0, 160.0]
                }]
            )
            .is_err()
    );
    assert!(
        workspace
            .check_revision(info.id, info.revision)
            .unwrap()
            .render_bytes()
            .is_ok()
    );
    workspace.set_draft(info.id, true).unwrap();
    workspace
        .cancel_image_replacement(info.id, asset.asset_id)
        .unwrap();
    assert!(
        workspace
            .ensure_export_ready(info.id, info.revision)
            .is_err()
    );
    workspace.set_draft(info.id, false).unwrap();
    assert!(
        workspace
            .ensure_export_ready(info.id, info.revision)
            .is_ok()
    );
}

fn encrypted(source: Document, version: u8, permissions: Permissions) -> Vec<u8> {
    let mut source = source;
    source.trailer.set(
        "ID",
        vec![
            Object::string_literal("image-synthetic-id"),
            Object::string_literal("image-synthetic-id"),
        ],
    );
    let version = if version == 1 {
        EncryptionVersion::V1 {
            document: &source,
            owner_password: "owner",
            user_password: "reader",
            permissions,
        }
    } else {
        EncryptionVersion::V2 {
            document: &source,
            owner_password: "owner",
            user_password: "reader",
            permissions,
            key_length: 128,
        }
    };
    let state = EncryptionState::try_from(version).unwrap();
    source.encrypt(&state).unwrap();
    bytes(source)
}

fn open_protected(workspace: &mut Workspace, bytes: Vec<u8>, password: &str) -> DocumentInfo {
    let OpenOutcome::PasswordRequired { request_id, .. } = workspace
        .request_open_bytes(bytes, "encrypted.pdf".into())
        .unwrap()
    else {
        panic!("expected password")
    };
    let OpenOutcome::Opened { document, .. } =
        workspace.unlock_document(request_id, password).unwrap()
    else {
        panic!("expected open")
    };
    document
}

#[test]
fn legacy_encrypted_image_mutations_preserve_passwords_file_id_save_reopen_and_history() {
    for version in [1, 2] {
        let original = encrypted(shared_nested(), version, Permissions::all());
        let mut workspace = Workspace::default();
        let info = open_protected(&mut workspace, original.clone(), "reader");
        let target = occurrences(&workspace, info.id, 0).remove(1);
        let asset = workspace
            .stage_image_replacement(info.id, info.revision, &rgba_png())
            .unwrap();
        let changed = apply(
            &mut workspace,
            info.id,
            ImageChange::Replace {
                page: 0,
                path: target.path,
                asset_id: asset.asset_id,
                placement: None,
            },
        );
        let bytes = workspace.document(info.id).unwrap().bytes();
        assert!(Document::load_mem(bytes).unwrap().is_encrypted());
        for password in ["owner", "reader"] {
            let source =
                Document::load_mem_with_options(&original, LoadOptions::with_password(password))
                    .unwrap();
            let saved =
                Document::load_mem_with_options(bytes, LoadOptions::with_password(password))
                    .unwrap();
            assert_eq!(
                source.trailer.get(b"ID").unwrap(),
                saved.trailer.get(b"ID").unwrap()
            );
            assert_eq!(
                source.extract_text(&[1, 2]).unwrap(),
                saved.extract_text(&[1, 2]).unwrap()
            );
            assert!(saved.was_encrypted());
        }
        workspace
            .current_mut(info.id, changed.revision)
            .unwrap()
            .undo()
            .unwrap();
        assert_eq!(workspace.document(info.id).unwrap().bytes(), original);
        workspace
            .current_mut(info.id, info.revision)
            .unwrap()
            .redo()
            .unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("encrypted-image.pdf");
        workspace
            .current_mut(info.id, changed.revision)
            .unwrap()
            .save_as(&path)
            .unwrap();
        let reopened = Session::from_bytes_with_password(
            std::fs::read(path).unwrap(),
            "encrypted.pdf".into(),
            None,
            99,
            Some("reader"),
        )
        .unwrap();
        assert!(reopened.info().unwrap().password_protected);
        assert_eq!(reopened.image_occurrences(0).unwrap().images.len(), 4);
    }
}

#[test]
fn restricted_and_signed_sources_are_inspectable_but_never_image_editable() {
    let mut workspace = Workspace::default();
    let info = open_protected(
        &mut workspace,
        encrypted(
            direct(b"q 80 0 0 60 80 140 cm /Img Do Q"),
            2,
            Permissions::COPYABLE | Permissions::FILLABLE,
        ),
        "reader",
    );
    let target = occurrences(&workspace, info.id, 0).remove(0);
    assert!(!target.editable);
    assert!(
        workspace
            .stage_image_replacement(info.id, info.revision, &rgba_png())
            .is_err()
    );
    assert!(
        workspace
            .edit_image_occurrence(
                info.id,
                info.revision,
                ImageChange::Remove {
                    page: 0,
                    path: target.path
                }
            )
            .is_err()
    );
    let mut signed = direct(b"q 80 0 0 60 80 140 cm /Img Do Q");
    let marker = signed.add_object(dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()], "Contents" => Object::string_literal("synthetic marker only") });
    signed
        .catalog_mut()
        .unwrap()
        .set("SyntheticSignature", marker);
    let (mut workspace, info) = open_workspace(signed);
    assert!(!occurrences(&workspace, info.id, 0)[0].editable);
    assert!(
        workspace
            .stage_image_replacement(info.id, info.revision, &rgba_png())
            .is_err()
    );
    assert!(
        workspace
            .edit_image_occurrence(
                info.id,
                info.revision,
                ImageChange::Place {
                    page: 0,
                    path: vec![2],
                    placement: [30.0, 40.0, 60.0, 80.0]
                }
            )
            .is_err()
    );
}

#[test]
fn write_original_synthetic_image_editing_fixtures_when_requested() {
    let Ok(folder) = std::env::var("FOLIO_IMAGE_EVIDENCE_DIR") else {
        return;
    };
    let folder = std::path::PathBuf::from(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let (mut workspace, info) = open_workspace(shared_nested());
    let original = workspace.document(info.id).unwrap().bytes().to_vec();
    let images = occurrences(&workspace, info.id, 0);
    std::fs::write(folder.join("source.pdf"), &original).unwrap();
    std::fs::write(folder.join("replacement.png"), rgba_png()).unwrap();
    std::fs::write(folder.join("oriented.jpg"), oriented_jpeg()).unwrap();
    let asset = workspace
        .stage_image_replacement(info.id, info.revision, &rgba_png())
        .unwrap();
    let changed = apply(
        &mut workspace,
        info.id,
        ImageChange::Replace {
            page: 0,
            path: images[1].path.clone(),
            asset_id: asset.asset_id,
            placement: None,
        },
    );
    std::fs::write(
        folder.join("replaced.pdf"),
        workspace.document(info.id).unwrap().bytes(),
    )
    .unwrap();
    workspace
        .current_mut(info.id, changed.revision)
        .unwrap()
        .undo()
        .unwrap();
    let changed = apply(
        &mut workspace,
        info.id,
        ImageChange::Remove {
            page: 0,
            path: images[1].path.clone(),
        },
    );
    std::fs::write(
        folder.join("removed.pdf"),
        workspace.document(info.id).unwrap().bytes(),
    )
    .unwrap();
    workspace
        .current_mut(info.id, changed.revision)
        .unwrap()
        .undo()
        .unwrap();
    apply(
        &mut workspace,
        info.id,
        ImageChange::Place {
            page: 0,
            path: images[0].path.clone(),
            placement: [70.0, 570.0, 60.0, 40.0],
        },
    );
    std::fs::write(
        folder.join("moved.pdf"),
        workspace.document(info.id).unwrap().bytes(),
    )
    .unwrap();
    std::fs::write(
        folder.join("fixture-contract.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source": info, "images": images, "replacementIndex": 1, "movedIndex": 0,
            "movedPlacement": [70.0, 570.0, 60.0, 40.0],
        }))
        .unwrap(),
    )
    .unwrap();

    let mut rotated = direct(b"q 80 0 0 60 80 140 cm /Img Do Q");
    let first = rotated.get_pages()[&1];
    let root = rotated
        .get_dictionary(first)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let template = rotated.get_dictionary(first).unwrap().clone();
    let mut pages = vec![];
    for (rotation, unit) in [(0, 1.0_f32), (90, 1.5), (180, 0.75), (270, 2.0)] {
        let mut page = template.clone();
        page.set("Rotate", rotation);
        page.set("UserUnit", unit);
        pages.push(Object::Reference(rotated.add_object(page)));
    }
    rotated.get_dictionary_mut(root).unwrap().set("Kids", pages);
    rotated.get_dictionary_mut(root).unwrap().set("Count", 4);
    let (mut workspace, info) = open_workspace(rotated);
    std::fs::write(
        folder.join("rotations-source.pdf"),
        workspace.document(info.id).unwrap().bytes(),
    )
    .unwrap();
    for page in 0..4 {
        let target = occurrences(&workspace, info.id, page).remove(0);
        apply(
            &mut workspace,
            info.id,
            ImageChange::Place {
                page,
                path: target.path,
                placement: [40.0, 60.0, 125.0, 75.0],
            },
        );
    }
    std::fs::write(
        folder.join("rotations-moved.pdf"),
        workspace.document(info.id).unwrap().bytes(),
    )
    .unwrap();
    std::fs::write(
        folder.join("rotations-info.json"),
        serde_json::to_vec_pretty(&current(&workspace, info.id)).unwrap(),
    )
    .unwrap();
}
