use std::cell::RefCell;

use kesharon_application::{
    ApplicationError, CancellationSignal, IdGenerator, OpenProject, OpenProjectCommand,
    RepositoryInspection, RepositoryService,
};

struct RecordingRepository {
    paths: RefCell<Vec<String>>,
    result: Result<RepositoryInspection, ApplicationError>,
}

impl RepositoryService for RecordingRepository {
    fn inspect(&self, requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        self.paths.borrow_mut().push(requested_path.to_owned());
        self.result.clone()
    }
}

struct FixedIds(&'static str);

impl IdGenerator for FixedIds {
    fn next_id(&self) -> String {
        self.0.to_owned()
    }
}

struct FixedCancellation(bool);

impl CancellationSignal for FixedCancellation {
    fn is_cancelled(&self) -> bool {
        self.0
    }
}

#[test]
fn open_project_uses_inspected_canonical_identity() {
    let repository = RecordingRepository {
        paths: RefCell::new(Vec::new()),
        result: Ok(RepositoryInspection {
            canonical_root: "C:\\canonical\\kesharon".into(),
            display_name: "Kesharon".into(),
        }),
    };
    let use_case = OpenProject::new(&repository, &FixedIds("project-1"));

    let project = use_case
        .execute(
            &OpenProjectCommand {
                requested_path: "C:\\code\\..\\canonical\\kesharon".into(),
                trusted: true,
            },
            &FixedCancellation(false),
        )
        .expect("the inspected repository is valid");

    assert_eq!(
        repository.paths.into_inner(),
        vec!["C:\\code\\..\\canonical\\kesharon"]
    );
    assert_eq!(project.id().as_str(), "project-1");
    assert_eq!(project.canonical_root(), "C:\\canonical\\kesharon");
    assert_eq!(project.display_name(), "Kesharon");
    assert!(project.is_trusted());
}

#[test]
fn repository_failure_is_returned_without_creating_a_project() {
    let repository = RecordingRepository {
        paths: RefCell::new(Vec::new()),
        result: Err(ApplicationError::Repository(
            "path is not a Git repository".into(),
        )),
    };
    let use_case = OpenProject::new(&repository, &FixedIds("project-2"));

    let result = use_case.execute(
        &OpenProjectCommand {
            requested_path: "C:\\not-a-repository".into(),
            trusted: false,
        },
        &FixedCancellation(false),
    );

    assert_eq!(
        result,
        Err(ApplicationError::Repository(
            "path is not a Git repository".into()
        ))
    );
}

#[test]
fn cancellation_before_inspection_preserves_repository_boundary() {
    let repository = RecordingRepository {
        paths: RefCell::new(Vec::new()),
        result: Ok(RepositoryInspection {
            canonical_root: "C:\\canonical\\kesharon".into(),
            display_name: "Kesharon".into(),
        }),
    };
    let use_case = OpenProject::new(&repository, &FixedIds("project-3"));

    let result = use_case.execute(
        &OpenProjectCommand {
            requested_path: "C:\\canonical\\kesharon".into(),
            trusted: false,
        },
        &FixedCancellation(true),
    );

    assert_eq!(result, Err(ApplicationError::Cancelled));
    assert!(repository.paths.into_inner().is_empty());
}

struct CancelsDuringInspection {
    cancelled: std::cell::Cell<bool>,
}

impl RepositoryService for CancelsDuringInspection {
    fn inspect(&self, _requested_path: &str) -> Result<RepositoryInspection, ApplicationError> {
        self.cancelled.set(true);
        Ok(RepositoryInspection {
            canonical_root: "C:\\canonical\\kesharon".into(),
            display_name: "Kesharon".into(),
        })
    }
}

impl CancellationSignal for CancelsDuringInspection {
    fn is_cancelled(&self) -> bool {
        self.cancelled.get()
    }
}

#[test]
fn cancellation_after_inspection_prevents_project_creation() {
    let boundary = CancelsDuringInspection {
        cancelled: std::cell::Cell::new(false),
    };
    let use_case = OpenProject::new(&boundary, &FixedIds("project-4"));

    let result = use_case.execute(
        &OpenProjectCommand {
            requested_path: "C:\\canonical\\kesharon".into(),
            trusted: false,
        },
        &boundary,
    );

    assert_eq!(result, Err(ApplicationError::Cancelled));
}
