use kesharon_domain::{Project, ProjectError, ProjectId};

#[test]
fn project_records_the_canonical_root_and_trust_decision() {
    let project = Project::new(
        ProjectId::new("project-1").expect("the fixture id is valid"),
        "Kesharon",
        "C:\\code\\kesharon",
        true,
    )
    .expect("the fixture project is valid");

    assert_eq!(project.id().as_str(), "project-1");
    assert_eq!(project.display_name(), "Kesharon");
    assert_eq!(project.canonical_root(), "C:\\code\\kesharon");
    assert!(project.is_trusted());
}

#[test]
fn project_rejects_blank_identity_fields() {
    assert_eq!(ProjectId::new(" "), Err(ProjectError::EmptyIdentifier));
    assert_eq!(
        Project::new(
            ProjectId::new("project-2").expect("the fixture id is valid"),
            "",
            "C:\\code\\kesharon",
            false,
        ),
        Err(ProjectError::EmptyDisplayName)
    );
    assert_eq!(
        Project::new(
            ProjectId::new("project-3").expect("the fixture id is valid"),
            "Kesharon",
            " ",
            false,
        ),
        Err(ProjectError::EmptyCanonicalRoot)
    );
}
