use std::path::Path;
use std::sync::Mutex;

use kesharon_application::{ApplicationError, StateRepository};
use kesharon_domain::{
    ExecutionMode, Project, ProjectId, ResourceBudget, Task, TaskCheckpoint, TaskId,
};
use rusqlite::{Connection, params};

pub struct SqliteStateRepository {
    connection: Mutex<Connection>,
}

impl SqliteStateRepository {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ApplicationError> {
        let conn =
            Connection::open(path).map_err(|err| ApplicationError::Storage(err.to_string()))?;
        Self::initialize(conn)
    }

    pub fn in_memory() -> Result<Self, ApplicationError> {
        let conn = Connection::open_in_memory()
            .map_err(|err| ApplicationError::Storage(err.to_string()))?;
        Self::initialize(conn)
    }

    fn initialize(conn: Connection) -> Result<Self, ApplicationError> {
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS projects (
                 id TEXT PRIMARY KEY,
                 display_name TEXT NOT NULL,
                 canonical_root TEXT NOT NULL,
                 trusted INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS tasks (
                 id TEXT PRIMARY KEY,
                 project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                 goal TEXT NOT NULL,
                 state TEXT NOT NULL,
                 execution_mode TEXT NOT NULL,
                 max_memory_bytes INTEGER NOT NULL,
                 max_disk_write_bytes INTEGER NOT NULL,
                 max_concurrent_tools INTEGER NOT NULL,
                 max_prompt_tokens INTEGER,
                 max_completion_tokens INTEGER,
                 max_cost_micros INTEGER,
                 is_active INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE IF NOT EXISTS task_checkpoints (
                 id TEXT PRIMARY KEY,
                 task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                 description TEXT NOT NULL,
                 git_ref TEXT NOT NULL,
                 timestamp_millis INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS idempotency_entries (
                 key TEXT PRIMARY KEY,
                 payload_hash TEXT NOT NULL,
                 response_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS session_metadata (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )
        .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        Ok(Self {
            connection: Mutex::new(conn),
        })
    }
}

impl StateRepository for SqliteStateRepository {
    fn save_project(&mut self, project: &Project) -> Result<(), ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        conn.execute(
            "INSERT INTO projects (id, display_name, canonical_root, trusted)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 display_name = excluded.display_name,
                 canonical_root = excluded.canonical_root,
                 trusted = excluded.trusted",
            params![
                project.id().as_str(),
                project.display_name(),
                project.canonical_root(),
                i64::from(project.is_trusted())
            ],
        )
        .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        conn.execute(
            "INSERT INTO session_metadata (key, value)
             VALUES ('last_active_project_id', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![project.id().as_str()],
        )
        .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        Ok(())
    }

    fn load_project(&self, id: &str) -> Result<Option<Project>, ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        let mut stmt = conn
            .prepare("SELECT id, display_name, canonical_root, trusted FROM projects WHERE id = ?1")
            .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        let mut rows = stmt
            .query(params![id])
            .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        if let Some(row) = rows
            .next()
            .map_err(|err| ApplicationError::Storage(err.to_string()))?
        {
            let id_str: String = row
                .get(0)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            let display_name: String = row
                .get(1)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            let canonical_root: String = row
                .get(2)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            let trusted: i64 = row
                .get(3)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;

            let project_id = ProjectId::new(id_str)
                .map_err(|err| ApplicationError::InvalidProject(err.to_string()))?;

            let project = Project::new(project_id, display_name, canonical_root, trusted != 0)
                .map_err(|err| ApplicationError::InvalidProject(err.to_string()))?;

            Ok(Some(project))
        } else {
            Ok(None)
        }
    }

    fn load_last_active_project(&self) -> Result<Option<Project>, ApplicationError> {
        let project_id_opt: Option<String> = {
            let conn = self
                .connection
                .lock()
                .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

            conn.query_row(
                "SELECT value FROM session_metadata WHERE key = 'last_active_project_id'",
                [],
                |row| row.get(0),
            )
            .ok()
        };

        match project_id_opt {
            Some(project_id) => self.load_project(&project_id),
            None => Ok(None),
        }
    }

    fn save_task(&mut self, task: &Task) -> Result<(), ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        let mode_str = match task.execution_mode() {
            ExecutionMode::Plan => "plan",
            ExecutionMode::Act => "act",
        };

        let existing_project_id: Option<String> = conn
            .query_row(
                "SELECT project_id FROM tasks WHERE id = ?1",
                params![task.id().as_str()],
                |row| row.get(0),
            )
            .ok();

        let project_id = if let Some(pid) = existing_project_id {
            pid
        } else {
            let last_project_id: Option<String> = conn
                .query_row(
                    "SELECT value FROM session_metadata WHERE key = 'last_active_project_id'",
                    [],
                    |row| row.get(0),
                )
                .ok();

            if let Some(pid) = last_project_id {
                pid
            } else {
                conn.query_row(
                    "SELECT id FROM projects ORDER BY rowid DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| {
                    ApplicationError::Storage(
                        "Cannot save task without an associated project".into(),
                    )
                })?
            }
        };

        let is_active = i64::from(task.is_active());

        conn.execute(
            "INSERT INTO tasks (
                 id, project_id, goal, state, execution_mode,
                 max_memory_bytes, max_disk_write_bytes, max_concurrent_tools,
                 max_prompt_tokens, max_completion_tokens, max_cost_micros, is_active
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                 goal = excluded.goal,
                 state = excluded.state,
                 execution_mode = excluded.execution_mode,
                 max_memory_bytes = excluded.max_memory_bytes,
                 max_disk_write_bytes = excluded.max_disk_write_bytes,
                 max_concurrent_tools = excluded.max_concurrent_tools,
                 max_prompt_tokens = excluded.max_prompt_tokens,
                 max_completion_tokens = excluded.max_completion_tokens,
                 max_cost_micros = excluded.max_cost_micros,
                 is_active = excluded.is_active",
            params![
                task.id().as_str(),
                project_id,
                task.goal(),
                format!("{:?}", task.state()),
                mode_str,
                i64_from_u64(task.budget().max_memory_bytes())?,
                i64_from_u64(task.budget().max_disk_write_bytes())?,
                i64::from(task.budget().max_concurrent_tools()),
                task.budget()
                    .max_prompt_tokens()
                    .map(i64_from_u64)
                    .transpose()?,
                task.budget()
                    .max_completion_tokens()
                    .map(i64_from_u64)
                    .transpose()?,
                task.budget()
                    .max_cost_micros()
                    .map(i64_from_u64)
                    .transpose()?,
                is_active,
            ],
        )
        .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        Ok(())
    }

    fn load_task(&self, id: &str) -> Result<Option<Task>, ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, goal, execution_mode, max_memory_bytes, max_disk_write_bytes,
                        max_concurrent_tools, max_prompt_tokens, max_completion_tokens, max_cost_micros
                 FROM tasks WHERE id = ?1",
            )
            .map_err(to_storage_err)?;

        let mut rows = stmt.query(params![id]).map_err(to_storage_err)?;
        match rows.next().map_err(to_storage_err)? {
            Some(row) => parse_task_row(row).map(Some),
            None => Ok(None),
        }
    }

    fn load_active_task(&self, project_id: &str) -> Result<Option<Task>, ApplicationError> {
        let task_id_opt: Option<String> = {
            let conn = self
                .connection
                .lock()
                .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

            conn.query_row(
                "SELECT id FROM tasks WHERE project_id = ?1 AND is_active = 1 ORDER BY rowid DESC LIMIT 1",
                params![project_id],
                |row| row.get(0),
            )
            .ok()
        };

        match task_id_opt {
            Some(id) => self.load_task(&id),
            None => Ok(None),
        }
    }

    fn save_checkpoint(
        &mut self,
        task_id: &str,
        checkpoint: &TaskCheckpoint,
    ) -> Result<(), ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        conn.execute(
            "INSERT INTO task_checkpoints (id, task_id, description, git_ref, timestamp_millis)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                 description = excluded.description,
                 git_ref = excluded.git_ref,
                 timestamp_millis = excluded.timestamp_millis",
            params![
                checkpoint.id(),
                task_id,
                checkpoint.description(),
                checkpoint.git_ref(),
                i64_from_u64(checkpoint.timestamp_millis())?,
            ],
        )
        .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        Ok(())
    }

    fn list_checkpoints(&self, task_id: &str) -> Result<Vec<TaskCheckpoint>, ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, description, git_ref, timestamp_millis
                 FROM task_checkpoints WHERE task_id = ?1 ORDER BY timestamp_millis ASC",
            )
            .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        let mut rows = stmt
            .query(params![task_id])
            .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|err| ApplicationError::Storage(err.to_string()))?
        {
            let id: String = row
                .get(0)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            let description: String = row
                .get(1)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            let git_ref: String = row
                .get(2)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            let ts: i64 = row
                .get(3)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;

            let ts_u64 = u64_from_i64(ts)?;
            let ckpt = TaskCheckpoint::new(id, description, git_ref, ts_u64)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            result.push(ckpt);
        }

        Ok(result)
    }

    fn record_idempotency(
        &mut self,
        key: &str,
        payload_hash: &str,
        response_json: &str,
    ) -> Result<(), ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        conn.execute(
            "INSERT INTO idempotency_entries (key, payload_hash, response_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
                 payload_hash = excluded.payload_hash,
                 response_json = excluded.response_json",
            params![key, payload_hash, response_json],
        )
        .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        Ok(())
    }

    fn find_idempotency(&self, key: &str) -> Result<Option<String>, ApplicationError> {
        let conn = self
            .connection
            .lock()
            .map_err(|_| ApplicationError::Storage("mutex poisoned".into()))?;

        let mut stmt = conn
            .prepare("SELECT response_json FROM idempotency_entries WHERE key = ?1")
            .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        let mut rows = stmt
            .query(params![key])
            .map_err(|err| ApplicationError::Storage(err.to_string()))?;

        if let Some(row) = rows
            .next()
            .map_err(|err| ApplicationError::Storage(err.to_string()))?
        {
            let resp: String = row
                .get(0)
                .map_err(|err| ApplicationError::Storage(err.to_string()))?;
            Ok(Some(resp))
        } else {
            Ok(None)
        }
    }
}

fn i64_from_u64(val: u64) -> Result<i64, ApplicationError> {
    i64::try_from(val).map_err(|err| ApplicationError::Storage(err.to_string()))
}

fn u64_from_i64(val: i64) -> Result<u64, ApplicationError> {
    u64::try_from(val).map_err(|err| ApplicationError::Storage(err.to_string()))
}

fn u16_from_i64(val: i64) -> Result<u16, ApplicationError> {
    u16::try_from(val).map_err(|err| ApplicationError::Storage(err.to_string()))
}

fn to_storage_err(err: impl std::fmt::Display) -> ApplicationError {
    ApplicationError::Storage(err.to_string())
}

struct RawTaskBudget {
    max_mem: i64,
    max_disk: i64,
    max_tools: i64,
    max_prompt: Option<i64>,
    max_completion: Option<i64>,
    max_cost: Option<i64>,
}

impl RawTaskBudget {
    fn into_resource_budget(self) -> Result<ResourceBudget, ApplicationError> {
        let mem = u64_from_i64(self.max_mem)?;
        let disk = u64_from_i64(self.max_disk)?;
        let tools = u16_from_i64(self.max_tools)?;
        let mut budget = ResourceBudget::new(mem, disk, tools).map_err(to_storage_err)?;

        if let (Some(p), Some(c), Some(cost)) =
            (self.max_prompt, self.max_completion, self.max_cost)
        {
            let p_u64 = u64_from_i64(p)?;
            let c_u64 = u64_from_i64(c)?;
            let cost_u64 = u64_from_i64(cost)?;
            budget = budget
                .with_token_limits(p_u64, c_u64, cost_u64)
                .map_err(to_storage_err)?;
        }

        Ok(budget)
    }
}

fn parse_task_row(row: &rusqlite::Row<'_>) -> Result<Task, ApplicationError> {
    let id_str: String = row.get(0).map_err(to_storage_err)?;
    let goal: String = row.get(1).map_err(to_storage_err)?;
    let mode_str: String = row.get(2).map_err(to_storage_err)?;
    let raw_budget = RawTaskBudget {
        max_mem: row.get(3).map_err(to_storage_err)?,
        max_disk: row.get(4).map_err(to_storage_err)?,
        max_tools: row.get(5).map_err(to_storage_err)?,
        max_prompt: row.get(6).map_err(to_storage_err)?,
        max_completion: row.get(7).map_err(to_storage_err)?,
        max_cost: row.get(8).map_err(to_storage_err)?,
    };

    let budget = raw_budget.into_resource_budget()?;
    let task_id = TaskId::new(id_str).map_err(to_storage_err)?;
    let mut task = Task::new(task_id, goal, budget).map_err(to_storage_err)?;

    if mode_str == "act" {
        task.set_execution_mode(ExecutionMode::Act);
    } else {
        task.set_execution_mode(ExecutionMode::Plan);
    }

    Ok(task)
}
