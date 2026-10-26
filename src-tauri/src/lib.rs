pub mod document;
mod pages;
mod protection;
pub mod redaction;
pub mod signatures;
pub mod text_edit;
pub mod workspace;
mod writing;

#[cfg(feature = "desktop")]
mod desktop;

#[cfg(feature = "desktop")]
pub use desktop::run;
