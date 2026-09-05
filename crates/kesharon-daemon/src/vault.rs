use std::collections::HashMap;
use std::sync::Mutex;

use kesharon_application::{ApplicationError, CredentialVault};

#[derive(Default)]
pub struct InMemoryCredentialVault {
    secrets: Mutex<HashMap<String, String>>,
}

impl InMemoryCredentialVault {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CredentialVault for InMemoryCredentialVault {
    fn get_secret(&self, key: &str) -> Result<Option<String>, ApplicationError> {
        let secrets = self
            .secrets
            .lock()
            .map_err(|err| ApplicationError::Vault(err.to_string()))?;
        Ok(secrets.get(key).cloned())
    }

    fn set_secret(&mut self, key: &str, secret: &str) -> Result<(), ApplicationError> {
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|err| ApplicationError::Vault(err.to_string()))?;
        secrets.insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete_secret(&mut self, key: &str) -> Result<bool, ApplicationError> {
        let mut secrets = self
            .secrets
            .lock()
            .map_err(|err| ApplicationError::Vault(err.to_string()))?;
        Ok(secrets.remove(key).is_some())
    }
}
