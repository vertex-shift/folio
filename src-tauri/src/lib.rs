pub mod document;
pub mod workspace;

#[cfg(feature = "desktop")]
mod desktop;

#[cfg(feature = "desktop")]
pub use desktop::run;
