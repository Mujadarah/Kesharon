use std::cell::RefCell;

use kesharon_application::{
    ApplicationError, IdGenerator, OpenProject, OpenProjectCommand, RepositoryInspection,
    RepositoryService,
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
        .execute(&OpenProjectCommand {
            requested_path: "C:\\code\\..\\canonical\\kesharon".into(),
            trusted: true,
        })
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

    let result = use_case.execute(&OpenProjectCommand {
        requested_path: "C:\\not-a-repository".into(),
        trusted: false,
    });

    assert_eq!(
        result,
        Err(ApplicationError::Repository(
            "path is not a Git repository".into()
        ))
    );
}
