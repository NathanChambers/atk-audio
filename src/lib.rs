pub mod edits;
pub mod project;
pub mod viewer;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
