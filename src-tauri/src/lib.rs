pub mod document;
mod pages;
pub mod workspace;
mod writing;

#[cfg(feature = "desktop")]
mod desktop;

#[cfg(feature = "desktop")]
pub use desktop::run;
