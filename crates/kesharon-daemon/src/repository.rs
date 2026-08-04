use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kesharon_application::{
    ApplicationError, IdGenerator, RepositoryInspection, RepositoryService,
};

pub(crate) fn production_repository() -> Arc<dyn RepositoryService + Send + Sync> {
    #[cfg(debug_assertions)]
    if let Some(barrier) = std::env::var_os("KESHARON_TEST_OPEN_BLOCK_UNTIL") {
        return Arc::new(BarrierRepository {
            barrier: PathBuf::from(barrier),
        });
    }
    Arc::new(FilesystemRepository)
}

pub(crate) struct UuidIds;

impl IdGenerator for UuidIds {
    fn next_id(&self) -> String {
        uuid::Uuid::now_v7().to_string()
    }
}

struct FilesystemRepository;

impl RepositoryService for FilesystemRepository {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        let root = std::fs::canonicalize(requested_path)
            .map_err(|_| ApplicationError::Repository("projectPathUnavailable".into()))?;
        let git_marker = root.join(".git");
        if !git_marker.is_file() && !git_marker.is_dir() {
            return Err(ApplicationError::Repository("notGitRepository".into()));
        }
        let display_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| ApplicationError::Repository("projectPathUnavailable".into()))?;
        Ok(RepositoryInspection {
            canonical_root: path_text(&root)?,
            display_name: display_name.to_owned(),
        })
    }
}

fn path_text(path: &Path) -> Result<String, ApplicationError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ApplicationError::Repository("projectPathUnavailable".into()))
}

#[cfg(debug_assertions)]
struct BarrierRepository {
    barrier: PathBuf,
}

#[cfg(debug_assertions)]
impl RepositoryService for BarrierRepository {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        std::fs::write(self.barrier.with_extension("entered"), b"entered")
            .map_err(|_| ApplicationError::Repository("projectPathUnavailable".into()))?;
        while !self.barrier.exists() {
            std::thread::sleep(Duration::from_millis(1));
        }
        FilesystemRepository.inspect(requested_path)
    }
}
