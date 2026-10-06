pub mod analysis;
pub mod bootstrap;
pub mod cafe;
pub mod context;
pub mod control_flow;
pub mod database;
pub mod function_validation;
mod image_audit;
pub mod ir;
pub mod jump_tables;
pub mod linking;
pub mod manmenmi_integration;
pub mod module_model;
pub mod native;
mod planning;
pub mod project;
pub mod query;
pub mod semantic;
mod semantic_flow;
pub mod semantic_interprocedural;
pub mod semantic_memory;
mod semantic_memory_reporting;
pub mod semantic_proof;
pub mod semantic_reconstruction;
mod semantic_reporting;
pub mod semantic_tables;
pub mod semantic_values;
pub mod snapshot;
pub use analysis::AnalysisService;
pub use project::{ProjectConfig, ProjectService};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("binary analysis: {0}")]
    Binary(#[from] woomy_binary::Error),
    #[error("configuration error: {0}")]
    Config(String),
    #[error("invalid project: {0}")]
    Project(String),
    #[error("analysis error: {0}")]
    Analysis(String),
    #[error("cancelled; completed module checkpoints are preserved")]
    Cancelled,
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
}
impl Error {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_) => 2,
            Self::Project(_) => 3,
            Self::Cancelled => 6,
            _ => 4,
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn hash_bytes(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

mod calls;
