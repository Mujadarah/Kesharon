use kesharon_application::CredentialVault;
use kesharon_daemon::InMemoryCredentialVault;

#[test]
fn in_memory_vault_stores_retrieves_and_deletes_secrets() {
    let mut vault = InMemoryCredentialVault::new();

    assert_eq!(
        vault.get_secret("anthropic_key").expect("get succeeds"),
        None
    );

    vault
        .set_secret("anthropic_key", "sk-ant-test-secret-12345")
        .expect("set succeeds");

    assert_eq!(
        vault.get_secret("anthropic_key").expect("get succeeds"),
        Some("sk-ant-test-secret-12345".to_string())
    );

    let deleted = vault
        .delete_secret("anthropic_key")
        .expect("delete succeeds");
    assert!(deleted);

    assert_eq!(
        vault.get_secret("anthropic_key").expect("get succeeds"),
        None
    );

    let deleted_again = vault
        .delete_secret("anthropic_key")
        .expect("delete again succeeds");
    assert!(!deleted_again);
}
