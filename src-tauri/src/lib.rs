pub mod accessibility;
pub mod archive;
pub mod attachments;
pub mod authoring;
mod authoring_image;
pub mod comparison;
pub mod conversion;
pub mod document;
pub mod forms;
pub mod image_edit;
pub mod metadata;
pub mod ocr;
pub mod outlines;
pub mod output;
pub mod page_geometry;
pub mod page_labels;
mod pages;
pub mod paragraph;
mod pdf_objects;
pub mod preflight;
pub mod protection;
pub mod recovery;
pub mod redaction;
pub mod signatures;
pub mod text_edit;
pub mod updates;
pub mod workspace;
mod writing;

#[cfg(feature = "desktop")]
mod desktop;

#[cfg(feature = "desktop")]
pub use desktop::run;
