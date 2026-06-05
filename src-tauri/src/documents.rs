use crate::models::{ConversionJob, DocumentOperation};
use anyhow::{anyhow, Context, Result};
use lopdf::{
    content::{Content, Operation},
    dictionary, Document, Object, ObjectId, Stream,
};
use std::{collections::BTreeMap, path::Path};

const PDF_POINTS_PER_PIXEL: f64 = 0.75;

pub fn run_document_job(job: &ConversionJob) -> Result<()> {
    match job.document_operation {
        Some(DocumentOperation::ImagesToPdf) => images_to_pdf(&job.input_paths, &job.output_path),
        Some(DocumentOperation::MergePdfs) => merge_pdfs(&job.input_paths, &job.output_path),
        None => Err(anyhow!("Missing document operation")),
    }
}

fn images_to_pdf(paths: &[String], output_path: &str) -> Result<()> {
    if paths.is_empty() {
        return Err(anyhow!("No images selected"));
    }

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut page_refs = Vec::new();

    for (index, path) in paths.iter().enumerate() {
        let image = image::open(path).with_context(|| format!("Failed to read image {}", file_name(path)))?;
        let rgb = image.to_rgb8();
        let width_px = rgb.width();
        let height_px = rgb.height();
        let page_width = (width_px as f64 * PDF_POINTS_PER_PIXEL).max(1.0);
        let page_height = (height_px as f64 * PDF_POINTS_PER_PIXEL).max(1.0);
        let image_name = format!("Im{}", index + 1);

        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => width_px as i64,
                "Height" => height_px as i64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            rgb.into_raw(),
        ));
        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! {
                image_name.as_str() => image_id,
            },
        });
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        Object::Real(page_width as f32),
                        0.into(),
                        0.into(),
                        Object::Real(page_height as f32),
                        0.into(),
                        0.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(image_name.into_bytes())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode()?));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(page_width as f32), Object::Real(page_height as f32)],
        });
        page_refs.push(page_id);
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_refs.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_refs.len() as i64,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    doc.compress();
    doc.save(output_path).context("Failed to save PDF")?;
    Ok(())
}

fn merge_pdfs(paths: &[String], output_path: &str) -> Result<()> {
    if paths.is_empty() {
        return Err(anyhow!("No PDFs selected"));
    }

    let mut max_id = 1;
    let mut documents_pages = BTreeMap::new();
    let mut documents_objects = BTreeMap::new();
    let mut document = Document::with_version("1.5");

    for path in paths {
        let mut doc = Document::load(path).with_context(|| format!("Failed to read PDF {}", file_name(path)))?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;

        for page_id in doc.get_pages().into_values() {
            let page = doc.get_object(page_id)?.to_owned();
            documents_pages.insert(page_id, page);
        }
        documents_objects.extend(doc.objects);
    }

    let mut catalog_object: Option<(ObjectId, Object)> = None;
    let mut pages_object: Option<(ObjectId, Object)> = None;

    for (object_id, object) in documents_objects {
        match object.type_name().unwrap_or(b"") {
            b"Catalog" => {
                catalog_object = Some((catalog_object.map(|(id, _)| id).unwrap_or(object_id), object));
            }
            b"Pages" => {
                if let Ok(dictionary) = object.as_dict() {
                    let mut dictionary = dictionary.clone();
                    if let Some((_, ref existing)) = pages_object {
                        if let Ok(existing_dictionary) = existing.as_dict() {
                            dictionary.extend(existing_dictionary);
                        }
                    }
                    pages_object = Some((pages_object.map(|(id, _)| id).unwrap_or(object_id), Object::Dictionary(dictionary)));
                }
            }
            b"Page" | b"Outlines" | b"Outline" => {}
            _ => {
                document.objects.insert(object_id, object);
            }
        }
    }

    let (page_id, page_object) = pages_object.ok_or_else(|| anyhow!("Pages root not found"))?;
    for (object_id, object) in documents_pages.iter() {
        if let Ok(dictionary) = object.as_dict() {
            let mut dictionary = dictionary.clone();
            dictionary.set("Parent", page_id);
            document.objects.insert(*object_id, Object::Dictionary(dictionary));
        }
    }

    let (catalog_id, catalog_object) = catalog_object.ok_or_else(|| anyhow!("Catalog root not found"))?;
    if let Ok(dictionary) = page_object.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Count", documents_pages.len() as u32);
        dictionary.set(
            "Kids",
            documents_pages.into_keys().map(Object::Reference).collect::<Vec<_>>(),
        );
        document.objects.insert(page_id, Object::Dictionary(dictionary));
    }
    if let Ok(dictionary) = catalog_object.as_dict() {
        let mut dictionary = dictionary.clone();
        dictionary.set("Pages", page_id);
        dictionary.remove(b"Outlines");
        document.objects.insert(catalog_id, Object::Dictionary(dictionary));
    }

    document.trailer.set("Root", catalog_id);
    document.max_id = document.objects.len() as u32;
    document.renumber_objects();
    document.compress();
    document.save(output_path).context("Failed to save merged PDF")?;
    Ok(())
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_string()
}
