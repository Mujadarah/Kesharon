#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{Display, Formatter};

use kesharon_domain::{Project, ProjectId};

pub trait RepositoryService {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError>;
}

pub trait IdGenerator {
    fn next_id(&self) -> String;
}

pub trait CancellationSignal {
    fn is_cancelled(&self) -> bool;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryInspection {
    pub canonical_root: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenProjectCommand {
    pub requested_path: String,
    pub trusted: bool,
}

pub struct OpenProject<'a, R: RepositoryService + ?Sized, I: IdGenerator + ?Sized> {
    repository: &'a R,
    ids: &'a I,
}

impl<'a, R: RepositoryService + ?Sized, I: IdGenerator + ?Sized> OpenProject<'a, R, I> {
    pub const fn new(repository: &'a R, ids: &'a I) -> Self {
        Self { repository, ids }
    }

    pub fn execute(
        &self,
        command: &OpenProjectCommand,
        cancellation: &impl CancellationSignal,
    ) -> Result<Project, ApplicationError> {
        if cancellation.is_cancelled() {
            return Err(ApplicationError::Cancelled);
        }
        let inspection = self.repository.inspect(&command.requested_path)?;
        if cancellation.is_cancelled() {
            return Err(ApplicationError::Cancelled);
        }
        let id = ProjectId::new(self.ids.next_id())
            .map_err(|error| ApplicationError::InvalidProject(error.to_string()))?;

        Project::new(
            id,
            inspection.display_name,
            inspection.canonical_root,
            command.trusted,
        )
        .map_err(|error| ApplicationError::InvalidProject(error.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationError {
    Repository(String),
    InvalidProject(String),
    Cancelled,
}

impl Display for ApplicationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository(message) => write!(formatter, "repository error: {message}"),
            Self::InvalidProject(message) => write!(formatter, "invalid project: {message}"),
            Self::Cancelled => formatter.write_str("operation was cancelled"),
        }
    }
}

impl Error for ApplicationError {}
