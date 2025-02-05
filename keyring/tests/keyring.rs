use keyring::{get_global_service_name, set_global_service_name, KeyringEntry};
use test_log::test;

#[test(tokio::test)]
async fn test_keyring_entry() {
    // test global keyring
    set_global_service_name("example");
    assert_eq!(get_global_service_name(), "example");
    set_global_service_name("example2");
    assert_eq!(get_global_service_name(), "example");

    // test entry
    let entry = KeyringEntry::try_new("key").unwrap();
    assert_eq!(entry.key, "key");

    // test set secret
    entry.set_secret("secret").await.unwrap();
    let secret = entry.get_secret().await.unwrap();
    assert_eq!(secret, "secret");

    // test delete entry
    entry.delete_secret().await.unwrap();
    assert_eq!(entry.find_secret().await.unwrap(), None);
}
