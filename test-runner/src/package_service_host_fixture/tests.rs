use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

use super::copy_fixture_tree;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn recursive_copy_fixture_tree_receipt_preserves_external_control_files() {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "skiff-host-fixture-copy-receipt-{}-{sequence}",
        std::process::id()
    ));
    let source = root.join("source");
    let target = root.join("target");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::write(source.join("http.yml"), "probe: { method: GET }\n").unwrap();
    fs::write(source.join("nested/websocket.yml"), "path: /socket\n").unwrap();
    fs::write(
        source.join("nested/source.skiff"),
        "function marker() -> bool { return true }\n",
    )
    .unwrap();

    copy_fixture_tree(&source, &target).unwrap();

    let receipt = ["http.yml", "nested/websocket.yml"].map(|path| {
        (
            path,
            fs::read(source.join(path)).unwrap(),
            fs::read(target.join(path)).unwrap(),
        )
    });
    assert!(
        receipt
            .iter()
            .all(|(_, source_bytes, copied_bytes)| source_bytes == copied_bytes),
        "recursive copy receipt must retain exact external control-file bytes"
    );
    assert_eq!(
        fs::read(target.join("nested/source.skiff")).unwrap(),
        fs::read(source.join("nested/source.skiff")).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn recursive_copy_rejects_symlinks_and_secures_secret_config() {
    let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "skiff-host-fixture-secret-copy-{}-{sequence}",
        std::process::id()
    ));
    let source = root.join("source");
    let target = root.join("target");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&source).unwrap();
    let secret = source.join("config.dev.secret.yml");
    fs::write(
        &secret,
        "\"example.com/service\": { apiKey: must-not-leak }\n",
    )
    .unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();

    let insecure = copy_fixture_tree(&source, &target).unwrap_err().to_string();
    assert!(insecure.contains("chmod 600"), "{insecure}");
    assert!(!insecure.contains("must-not-leak"), "{insecure}");
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    copy_fixture_tree(&source, &target).unwrap();
    assert_eq!(
        fs::metadata(target.join("config.dev.secret.yml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    let linked = source.join("linked.skiff");
    symlink(&secret, &linked).unwrap();
    let error = copy_fixture_tree(&source, &target).unwrap_err().to_string();
    assert!(error.contains("contains symlink"), "{error}");
    assert!(!error.contains("must-not-leak"), "{error}");
    fs::remove_dir_all(root).unwrap();
}
