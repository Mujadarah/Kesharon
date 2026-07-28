use crate::TaskError;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, TaskError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(TaskError::EmptyIdentifier);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

identifier!(TaskId);
identifier!(TaskStepId);
