use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ProjectId(String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProjectError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ProjectError::EmptyIdentifier);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Project {
    id: ProjectId,
    display_name: String,
    canonical_root: String,
    trusted: bool,
}

impl Project {
    pub fn new(
        id: ProjectId,
        display_name: impl Into<String>,
        canonical_root: impl Into<String>,
        trusted: bool,
    ) -> Result<Self, ProjectError> {
        let display_name = display_name.into();
        if display_name.trim().is_empty() {
            return Err(ProjectError::EmptyDisplayName);
        }

        let canonical_root = canonical_root.into();
        if canonical_root.trim().is_empty() {
            return Err(ProjectError::EmptyCanonicalRoot);
        }

        Ok(Self {
            id,
            display_name,
            canonical_root,
            trusted,
        })
    }

    pub const fn id(&self) -> &ProjectId {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn canonical_root(&self) -> &str {
        &self.canonical_root
    }

    pub const fn is_trusted(&self) -> bool {
        self.trusted
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectError {
    EmptyIdentifier,
    EmptyDisplayName,
    EmptyCanonicalRoot,
}

impl Display for ProjectError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyIdentifier => formatter.write_str("project identifier must not be blank"),
            Self::EmptyDisplayName => formatter.write_str("project display name must not be blank"),
            Self::EmptyCanonicalRoot => {
                formatter.write_str("project canonical root must not be blank")
            }
        }
    }
}

impl Error for ProjectError {}
