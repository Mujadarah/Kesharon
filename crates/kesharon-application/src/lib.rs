#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{Display, Formatter};

use kesharon_domain::{Project, ProjectId, Task, TaskCheckpoint};

pub trait RepositoryService {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError>;
}

pub trait IdGenerator {
    fn next_id(&self) -> String;
}

pub trait CancellationSignal {
    fn is_cancelled(&self) -> bool;
}

pub trait StateRepository {
    fn save_project(&mut self, project: &Project) -> Result<(), ApplicationError>;
    fn load_project(&self, id: &str) -> Result<Option<Project>, ApplicationError>;
    fn load_last_active_project(&self) -> Result<Option<Project>, ApplicationError>;
    fn save_task(&mut self, task: &Task) -> Result<(), ApplicationError>;
    fn load_task(&self, id: &str) -> Result<Option<Task>, ApplicationError>;
    fn save_checkpoint(
        &mut self,
        task_id: &str,
        checkpoint: &TaskCheckpoint,
    ) -> Result<(), ApplicationError>;
    fn list_checkpoints(&self, task_id: &str) -> Result<Vec<TaskCheckpoint>, ApplicationError>;
    fn record_idempotency(
        &mut self,
        key: &str,
        payload_hash: &str,
        response_json: &str,
    ) -> Result<(), ApplicationError>;
    fn find_idempotency(&self, key: &str) -> Result<Option<String>, ApplicationError>;
}

pub trait CredentialVault {
    fn get_secret(&self, key: &str) -> Result<Option<String>, ApplicationError>;
    fn set_secret(&mut self, key: &str, secret: &str) -> Result<(), ApplicationError>;
    fn delete_secret(&mut self, key: &str) -> Result<bool, ApplicationError>;
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveredSession {
    project: Option<Project>,
    active_task: Option<Task>,
}

impl RecoveredSession {
    pub const fn new(project: Option<Project>, active_task: Option<Task>) -> Self {
        Self {
            project,
            active_task,
        }
    }

    pub const fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }

    pub const fn active_task(&self) -> Option<&Task> {
        self.active_task.as_ref()
    }
}

pub struct SessionRecovery<'a, S: StateRepository + ?Sized> {
    state_repository: &'a S,
}

impl<'a, S: StateRepository + ?Sized> SessionRecovery<'a, S> {
    pub const fn new(state_repository: &'a S) -> Self {
        Self { state_repository }
    }

    pub fn execute(&self) -> Result<RecoveredSession, ApplicationError> {
        let project = self.state_repository.load_last_active_project()?;
        Ok(RecoveredSession::new(project, None))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationError {
    Repository(String),
    InvalidProject(String),
    Storage(String),
    Vault(String),
    Cancelled,
}

impl Display for ApplicationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Repository(message) => write!(formatter, "repository error: {message}"),
            Self::InvalidProject(message) => write!(formatter, "invalid project: {message}"),
            Self::Storage(message) => write!(formatter, "storage error: {message}"),
            Self::Vault(message) => write!(formatter, "vault error: {message}"),
            Self::Cancelled => formatter.write_str("operation was cancelled"),
        }
    }
}

impl Error for ApplicationError {}
