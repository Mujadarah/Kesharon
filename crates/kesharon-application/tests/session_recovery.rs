use kesharon_application::{
    ApplicationError, CredentialVault, RecoveredSession, SessionRecovery, StateRepository,
};
use kesharon_domain::{Project, ProjectId, Task, TaskCheckpoint};
use std::collections::HashMap;

#[derive(Default)]
struct InMemoryStateRepo {
    projects: HashMap<String, Project>,
    tasks: HashMap<String, Task>,
    checkpoints: HashMap<String, Vec<TaskCheckpoint>>,
    idempotency: HashMap<String, String>,
    last_active_project_id: Option<String>,
}

impl StateRepository for InMemoryStateRepo {
    fn save_project(&mut self, project: &Project) -> Result<(), ApplicationError> {
        self.projects
            .insert(project.id().as_str().to_string(), project.clone());
        self.last_active_project_id = Some(project.id().as_str().to_string());
        Ok(())
    }

    fn load_project(&self, id: &str) -> Result<Option<Project>, ApplicationError> {
        Ok(self.projects.get(id).cloned())
    }

    fn load_last_active_project(&self) -> Result<Option<Project>, ApplicationError> {
        match &self.last_active_project_id {
            Some(id) => self.load_project(id),
            None => Ok(None),
        }
    }

    fn save_task(&mut self, task: &Task) -> Result<(), ApplicationError> {
        self.tasks
            .insert(task.id().as_str().to_string(), task.clone());
        Ok(())
    }

    fn load_task(&self, id: &str) -> Result<Option<Task>, ApplicationError> {
        Ok(self.tasks.get(id).cloned())
    }

    fn save_checkpoint(
        &mut self,
        task_id: &str,
        checkpoint: &TaskCheckpoint,
    ) -> Result<(), ApplicationError> {
        self.checkpoints
            .entry(task_id.to_string())
            .or_default()
            .push(checkpoint.clone());
        Ok(())
    }

    fn list_checkpoints(&self, task_id: &str) -> Result<Vec<TaskCheckpoint>, ApplicationError> {
        Ok(self.checkpoints.get(task_id).cloned().unwrap_or_default())
    }

    fn record_idempotency(
        &mut self,
        key: &str,
        _payload_hash: &str,
        response_json: &str,
    ) -> Result<(), ApplicationError> {
        self.idempotency
            .insert(key.to_string(), response_json.to_string());
        Ok(())
    }

    fn find_idempotency(&self, key: &str) -> Result<Option<String>, ApplicationError> {
        Ok(self.idempotency.get(key).cloned())
    }
}

#[derive(Default)]
struct InMemoryVault {
    secrets: HashMap<String, String>,
}

impl CredentialVault for InMemoryVault {
    fn get_secret(&self, key: &str) -> Result<Option<String>, ApplicationError> {
        Ok(self.secrets.get(key).cloned())
    }

    fn set_secret(&mut self, key: &str, secret: &str) -> Result<(), ApplicationError> {
        self.secrets.insert(key.to_string(), secret.to_string());
        Ok(())
    }

    fn delete_secret(&mut self, key: &str) -> Result<bool, ApplicationError> {
        Ok(self.secrets.remove(key).is_some())
    }
}

#[test]
fn session_recovery_restores_last_active_project() {
    let mut repo = InMemoryStateRepo::default();

    let project = Project::new(
        ProjectId::new("proj-1").expect("valid id"),
        "Kesharon",
        "/workspace/kesharon",
        true,
    )
    .expect("valid project");
    repo.save_project(&project).expect("saved project");

    let recovery = SessionRecovery::new(&repo);
    let recovered: RecoveredSession = recovery.execute().expect("recovery succeeds");

    assert_eq!(
        recovered.project().map(Project::display_name),
        Some("Kesharon")
    );
}

#[test]
fn credential_vault_stores_retrieves_and_deletes_secrets() {
    let mut vault = InMemoryVault::default();

    vault
        .set_secret("openai_api_key", "sk-test-secret")
        .expect("vault set");

    assert_eq!(
        vault.get_secret("openai_api_key").expect("vault get"),
        Some("sk-test-secret".to_string())
    );

    assert!(vault.delete_secret("openai_api_key").expect("vault delete"));
    assert_eq!(vault.get_secret("openai_api_key").expect("vault get"), None);
}

#[test]
fn checkpoint_lifecycle_records_and_retrieves_checkpoints() {
    let mut repo = InMemoryStateRepo::default();
    let chk = TaskCheckpoint::new(
        "chk-001",
        "Before refactor",
        "refs/heads/checkpoint-001",
        1_700_000_000,
    )
    .expect("valid checkpoint");

    repo.save_checkpoint("task-1", &chk).expect("saved");
    let list = repo.list_checkpoints("task-1").expect("listed");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id(), "chk-001");
}
