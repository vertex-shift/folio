use folio_core::{
    document::{Edit, Session},
    redaction::{PixelRect, Plan, Region},
    workspace::Workspace,
};
use lopdf::{Document, Object, Stream, dictionary};

fn source(rotation: i32, unit: f32) -> Vec<u8> {
    let mut pdf = Document::with_version("1.7");
    let pages = pdf.new_object_id();
    let font = pdf.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let content = pdf.add_object(Stream::new(
        dictionary! {},
        b"BT /F1 12 Tf 12 60 Td (CONFIDENTIAL_SOURCE_TEXT) Tj ET".to_vec(),
    ));
    let note = pdf.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text",
        "Rect" => vec![1.into(), 1.into(), 12.into(), 12.into()],
        "Contents" => lopdf::text_string("CONFIDENTIAL_NOTE_BODY"),
    });
    let ids: Vec<_> = (0..2)
        .map(|_| {
            pdf.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages, "Contents" => content,
                "Rotate" => rotation, "UserUnit" => unit, "Annots" => vec![Object::Reference(note)],
            })
        })
        .collect();
    pdf.objects.insert(
        pages,
        dictionary! {
            "Type" => "Pages", "Count" => 2,
            "Kids" => ids.into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "MediaBox" => vec![(-10).into(), (-20).into(), 100.into(), 120.into()],
            "CropBox" => vec![10.into(), 20.into(), 82.into(), 116.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        }
        .into(),
    );
    let metadata = pdf.add_object(Stream::new(
        dictionary! {
            "Type" => "Metadata", "Subtype" => "XML",
        },
        b"CONFIDENTIAL_XMP_METADATA".to_vec(),
    ));
    let attachment = pdf.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"CONFIDENTIAL_ATTACHMENT_BYTES".to_vec(),
    ));
    let file = pdf.add_object(dictionary! {
        "Type" => "Filespec", "F" => lopdf::text_string("private.txt"),
        "EF" => dictionary! { "F" => attachment },
    });
    let root = pdf.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages, "Metadata" => metadata,
        "Names" => dictionary! { "EmbeddedFiles" => dictionary! {
            "Names" => vec![lopdf::text_string("private.txt"), Object::Reference(file)],
        }},
        "OpenAction" => dictionary! { "S" => "JavaScript", "JS" => lopdf::text_string("CONFIDENTIAL_SCRIPT") },
    });
    let info =
        pdf.add_object(dictionary! { "Author" => lopdf::text_string("CONFIDENTIAL_AUTHOR") });
    pdf.trailer.set("Root", root);
    pdf.trailer.set("Info", info);
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

fn workspace(rotation: i32, unit: f32) -> Workspace {
    let mut workspace = Workspace::default();
    workspace
        .open_bytes(source(rotation, unit), "Private original.pdf".into(), false)
        .unwrap();
    workspace
}

fn region() -> Region {
    Region {
        page: 0,
        rect: [22.0, 44.0, 46.0, 80.0],
    }
}

fn begin(workspace: &mut Workspace) -> Plan {
    let info = workspace.current().unwrap().info().unwrap();
    workspace
        .begin_redaction(info.id, info.revision, 150, &[region()])
        .unwrap()
}

fn png(width: u32, height: u32, alpha: Option<u8>) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(if alpha.is_some() {
            png::ColorType::Rgba
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_text_chunk("Comment".into(), "CONFIDENTIAL_PNG_METADATA".into())
            .unwrap();
        let mut writer = encoder.write_header().unwrap();
        let pixel = match alpha {
            Some(alpha) => vec![17, 41, 91, alpha],
            None => vec![17, 41, 91],
        };
        writer
            .write_image_data(&pixel.repeat(width as usize * height as usize))
            .unwrap();
    }
    bytes
}

fn stage_all(workspace: &mut Workspace, plan: &Plan, alpha: Option<u8>) {
    for (index, page) in plan.pages.iter().enumerate() {
        workspace
            .stage_redaction_page(
                plan.job_id,
                index,
                &png(page.pixel_width, page.pixel_height, alpha),
            )
            .unwrap();
    }
}

fn images(pdf: &Document) -> Vec<&Stream> {
    pdf.objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter(|stream| {
            stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
        })
        .collect()
}

#[test]
fn burns_actual_pixels_and_exports_no_original_pdf_objects_or_hidden_content() {
    let mut workspace = workspace(0, 1.0);
    let info = workspace.current().unwrap().info().unwrap();
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .apply(Edit::Note {
            page: 0,
            point: [30.0, 70.0],
            text: "An unsaved original note".into(),
        })
        .unwrap();
    let before = workspace.current().unwrap().bytes().to_vec();
    let before_info = serde_json::to_value(workspace.current().unwrap().info().unwrap()).unwrap();
    let plan = begin(&mut workspace);
    stage_all(&mut workspace, &plan, None);
    let bytes = workspace.redaction_bytes(plan.job_id).unwrap();
    let pdf = Document::load_mem(&bytes).unwrap();
    assert_eq!(pdf.get_pages().len(), 2);
    assert!(pdf.extract_text(&[1, 2]).unwrap().trim().is_empty());
    let catalog = pdf.catalog().unwrap();
    for key in [
        b"Metadata".as_slice(),
        b"Names",
        b"AcroForm",
        b"Outlines",
        b"OpenAction",
        b"StructTreeRoot",
    ] {
        assert!(!catalog.has(key));
    }
    let info_id = pdf.trailer.get(b"Info").unwrap().as_reference().unwrap();
    let info = pdf.get_dictionary(info_id).unwrap();
    assert_eq!(info.len(), 1);
    assert!(info.has(b"Producer"));
    assert!(
        !pdf.objects
            .values()
            .any(|object| object.as_dict().is_ok_and(|dict| dict.has(b"Font")))
    );
    for id in pdf.get_pages().values() {
        assert!(!pdf.get_dictionary(*id).unwrap().has(b"Annots"));
    }
    let images = images(&pdf);
    assert_eq!(images.len(), 2);
    for (index, image) in images.iter().enumerate() {
        assert_eq!(
            image.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
            b"DeviceRGB"
        );
        assert!(!image.dict.has(b"SMask"));
        let pixels = image.decompressed_content().unwrap();
        let page = &plan.pages[index];
        assert_eq!(
            pixels.len(),
            page.pixel_width as usize * page.pixel_height as usize * 3
        );
        for y in 0..page.pixel_height {
            for x in 0..page.pixel_width {
                let covered = page.masks.iter().any(|mask| {
                    x >= mask.x
                        && x < mask.x + mask.width
                        && y >= mask.y
                        && y < mask.y + mask.height
                });
                let start = (y as usize * page.pixel_width as usize + x as usize) * 3;
                assert_eq!(
                    &pixels[start..start + 3],
                    if covered { &[0, 0, 0] } else { &[17, 41, 91] }
                );
            }
        }
    }
    for object in pdf.objects.values() {
        if let Ok(stream) = object.as_stream() {
            assert!(
                !stream
                    .decompressed_content()
                    .unwrap_or_else(|_| stream.content.clone())
                    .windows(b"CONFIDENTIAL".len())
                    .any(|window| window == b"CONFIDENTIAL")
            );
        }
    }
    assert_eq!(workspace.current().unwrap().bytes(), before);
    assert_eq!(
        serde_json::to_value(workspace.current().unwrap().info().unwrap()).unwrap(),
        before_info
    );
    let folder = tempfile::tempdir().unwrap();
    let output = folder.path().join("Redacted.pdf");
    folio_core::document::atomic_write(&output, &bytes).unwrap();
    let reopened = Session::from_path(output, 2).unwrap();
    assert!(reopened.info().unwrap().annotations.is_empty());
    assert!(reopened.text_runs(0).unwrap().is_empty());
}

#[test]
fn rotation_crop_offsets_and_user_units_map_to_the_correct_pixels() {
    for (rotation, expected) in [
        (
            0,
            PixelRect {
                x: 49,
                y: 149,
                width: 102,
                height: 152,
            },
        ),
        (
            90,
            PixelRect {
                x: 99,
                y: 49,
                width: 152,
                height: 102,
            },
        ),
        (
            180,
            PixelRect {
                x: 149,
                y: 99,
                width: 102,
                height: 152,
            },
        ),
        (
            270,
            PixelRect {
                x: 149,
                y: 149,
                width: 152,
                height: 102,
            },
        ),
    ] {
        let mut workspace = workspace(rotation, 2.0);
        let plan = begin(&mut workspace);
        let page = &plan.pages[0];
        assert_eq!(page.source_bounds, [10.0, 20.0, 82.0, 116.0]);
        assert_eq!(page.masks, vec![expected]);
        assert_eq!(
            (page.pixel_width, page.pixel_height),
            if rotation % 180 == 0 {
                (300, 400)
            } else {
                (400, 300)
            }
        );
        stage_all(&mut workspace, &plan, None);
        let pdf = Document::load_mem(&workspace.redaction_bytes(plan.job_id).unwrap()).unwrap();
        let first = *pdf.get_pages().values().next().unwrap();
        let bounds = pdf
            .get_dictionary(first)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(bounds[2].as_float().unwrap() as f64, page.width_points);
        assert_eq!(bounds[3].as_float().unwrap() as f64, page.height_points);
        assert!(!pdf.get_dictionary(first).unwrap().has(b"Rotate"));
    }
}

#[test]
fn transparent_hidden_rgb_channels_are_discarded_not_embedded() {
    for (alpha, expected) in [
        (0, [255, 255, 255]),
        (128, [136, 148, 173]),
        (255, [17, 41, 91]),
    ] {
        let mut workspace = workspace(0, 1.0);
        let plan = begin(&mut workspace);
        stage_all(&mut workspace, &plan, Some(alpha));
        let pdf = Document::load_mem(&workspace.redaction_bytes(plan.job_id).unwrap()).unwrap();
        let image = images(&pdf)[0];
        let pixels = image.decompressed_content().unwrap();
        assert_eq!(&pixels[..3], &expected);
        let mask = &plan.pages[0].masks[0];
        let start = (mask.y as usize * plan.pages[0].pixel_width as usize + mask.x as usize) * 3;
        assert_eq!(&pixels[start..start + 3], &[0, 0, 0]);
    }
}

#[test]
fn refuses_partial_out_of_order_corrupt_or_wrong_sized_page_exports() {
    let mut workspace = workspace(0, 1.0);
    let plan = begin(&mut workspace);
    assert!(workspace.redaction_bytes(plan.job_id).is_err());
    assert!(
        workspace
            .stage_redaction_page(plan.job_id, 1, b"invalid")
            .is_err()
    );
    assert!(
        workspace
            .stage_redaction_page(plan.job_id, 0, b"invalid")
            .is_err()
    );
    let oversized = vec![0; folio_core::redaction::MAX_PAGE_PNG_BYTES + 1];
    assert!(
        workspace
            .stage_redaction_page(plan.job_id, 0, &oversized)
            .unwrap_err()
            .to_string()
            .contains("64 MiB")
    );
    assert!(
        workspace
            .stage_redaction_page(plan.job_id, 0, &png(1, 1, None))
            .is_err()
    );
    let page = &plan.pages[0];
    let png = png(page.pixel_width, page.pixel_height, None);
    workspace
        .stage_redaction_page(plan.job_id, 0, &png)
        .unwrap();
    assert!(
        workspace
            .stage_redaction_page(plan.job_id, 0, &png)
            .is_err()
    );
    assert!(workspace.redaction_bytes(plan.job_id).is_err());
    let second = &plan.pages[1];
    workspace
        .stage_redaction_page(
            plan.job_id,
            1,
            &self::png(second.pixel_width, second.pixel_height, None),
        )
        .unwrap();
    assert!(workspace.redaction_bytes(plan.job_id).is_ok());
}

#[test]
fn stale_and_superseded_jobs_cannot_finish_and_cancel_never_changes_the_source() {
    let mut workspace = workspace(0, 1.0);
    let before = workspace.current().unwrap().bytes().to_vec();
    let old = begin(&mut workspace);
    let new = begin(&mut workspace);
    assert_ne!(old.job_id, new.job_id);
    assert!(workspace.redaction_bytes(old.job_id).is_err());
    assert!(!workspace.cancel_redaction(old.job_id));
    assert!(workspace.check_redaction(new.job_id).is_ok());
    assert!(workspace.cancel_redaction(new.job_id));
    assert!(!workspace.cancel_redaction(new.job_id));
    assert_eq!(workspace.current().unwrap().bytes(), before);
    let plan = begin(&mut workspace);
    stage_all(&mut workspace, &plan, None);
    let info = workspace.current().unwrap().info().unwrap();
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(workspace.redaction_bytes(plan.job_id).is_err());
    assert!(
        workspace
            .stage_redaction_page(plan.job_id, 0, b"invalid")
            .is_err()
    );
    assert!(
        workspace
            .begin_redaction(info.id, info.revision, 150, &[region()])
            .is_err()
    );
}

#[test]
fn rejects_invalid_regions_resolution_and_unsupported_signed_or_xfa_sources() {
    let mut workspace = workspace(0, 1.0);
    let info = workspace.current().unwrap().info().unwrap();
    for regions in [
        vec![],
        vec![Region {
            page: 2,
            rect: region().rect,
        }],
        vec![Region {
            page: 0,
            rect: [1.0, 1.0, 2.0, 2.0],
        }],
        vec![Region {
            page: 0,
            rect: [22.0, 44.0, 22.0, 50.0],
        }],
        vec![Region {
            page: 0,
            rect: [22.0, 44.0, f64::NAN, 50.0],
        }],
        vec![region(); 501],
    ] {
        assert!(
            workspace
                .begin_redaction(info.id, info.revision, 150, &regions)
                .is_err()
        );
    }
    assert!(
        workspace
            .begin_redaction(info.id, info.revision, 72, &[region()])
            .is_err()
    );
    for signed in [false, true] {
        let mut pdf = Document::load_mem(&source(0, 1.0)).unwrap();
        if signed {
            pdf.add_object(
                dictionary! { "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into()] },
            );
        } else {
            let root = pdf.trailer.get(b"Root").unwrap().as_reference().unwrap();
            pdf.get_dictionary_mut(root).unwrap().set(
                "AcroForm",
                dictionary! { "XFA" => lopdf::text_string("xfa") },
            );
        }
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).unwrap();
        workspace
            .open_bytes(bytes, "unsupported.pdf".into(), true)
            .unwrap();
        let info = workspace.current().unwrap().info().unwrap();
        let message = workspace
            .begin_redaction(info.id, info.revision, 150, &[region()])
            .unwrap_err()
            .to_string();
        assert!(message.contains(if signed { "Digitally signed" } else { "XFA" }));
    }
}

#[test]
fn enforces_the_actual_pixel_limit_without_silent_resolution_fallback() {
    for (height, allowed) in [(2304, true), (2400, false)] {
        let mut pdf = Document::load_mem(&source(0, 1.0)).unwrap();
        for page in pdf.get_pages().values() {
            let page = pdf.get_dictionary_mut(*page).unwrap();
            page.set(
                "MediaBox",
                vec![0.into(), 0.into(), 2400.into(), height.into()],
            );
            page.set(
                "CropBox",
                vec![0.into(), 0.into(), 2400.into(), height.into()],
            );
        }
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).unwrap();
        let mut workspace = Workspace::default();
        let info = workspace
            .open_bytes(bytes, "large.pdf".into(), false)
            .unwrap();
        let result = workspace.begin_redaction(info.id, info.revision, 150, &[region()]);
        assert_eq!(result.is_ok(), allowed);
        if let Ok(plan) = result {
            assert_eq!(
                u64::from(plan.pages[0].pixel_width) * u64::from(plan.pages[0].pixel_height),
                24_000_000
            );
            assert_eq!(plan.dpi, 150);
        }
        assert!(
            workspace
                .begin_redaction(info.id, info.revision, 300, &[region()])
                .is_err()
        );
    }
    let mut pdf = Document::load_mem(&source(0, 1.0)).unwrap();
    for page in pdf.get_pages().values() {
        let page = pdf.get_dictionary_mut(*page).unwrap();
        page.set(
            "MediaBox",
            vec![0.into(), 0.into(), 8000.into(), 100.into()],
        );
        page.set("CropBox", vec![0.into(), 0.into(), 8000.into(), 100.into()]);
    }
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace
        .open_bytes(bytes, "wide.pdf".into(), false)
        .unwrap();
    assert!(
        workspace
            .begin_redaction(info.id, info.revision, 150, &[region()])
            .is_err()
    );
}

#[test]
fn protects_original_saved_copy_and_symlink_targets_from_native_export() {
    let folder = tempfile::tempdir().unwrap();
    let original = folder.path().join("original.pdf");
    std::fs::write(&original, source(0, 1.0)).unwrap();
    let mut workspace = Workspace::default();
    let info = workspace.open_path(original.clone(), false).unwrap();
    let saved = folder.path().join("edited.pdf");
    workspace
        .current_mut(info.id, info.revision)
        .unwrap()
        .save_as(&saved)
        .unwrap();
    let plan = begin(&mut workspace);
    let session = workspace.check_redaction(plan.job_id).unwrap();
    assert!(session.ensure_separate_output(&original).is_err());
    assert!(session.ensure_separate_output(&saved).is_err());
    assert!(
        session
            .ensure_separate_output(&folder.path().join("Redacted.pdf"))
            .is_ok()
    );
    #[cfg(unix)]
    {
        let alias = folder.path().join("alias.pdf");
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        assert!(session.ensure_separate_output(&alias).is_err());
    }
}
