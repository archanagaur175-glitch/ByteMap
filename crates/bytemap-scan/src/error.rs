//! Scanner error types.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("cannot access scan root {0:?}: {1}")]
    Root(PathBuf, std::io::Error),
    #[error("walk error{maybe}: {0}", maybe = .0.path().map(|p| format!(" near {p:?}")).unwrap_or_default())]
    Walk(#[from] jwalk::Error),
    #[error("internal error: parent directory {0:?} was not registered before its children")]
    MissingParent(PathBuf),
}
