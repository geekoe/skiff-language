use std::{
    fs,
    sync::{Arc, Barrier},
};

use serde_json::json;
use skiff_artifact_identity::{
    runtime_assembly_ref, EnvironmentActivationStatePath, RuntimeAssemblyRecordPath,
};
use skiff_deployment::{assembly::resolve_runtime_assembly, storage::CanonicalArtifactStore};

use super::{invoke, TestRoot};

#[test]
fn ecosystem_store_concurrent_bootstrap_converges_on_one_state() {
    let root = TestRoot::new("concurrent-bootstrap");
    let worker_count = 8;
    let barrier = Arc::new(Barrier::new(worker_count));
    let handles = (0..worker_count)
        .map(|_| {
            let path = root.path().to_path_buf();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                invoke(
                    &path,
                    json!({
                        "operation": "ensureEnvironmentBootstrap",
                        "environment": "test"
                    }),
                )
            })
        })
        .collect::<Vec<_>>();

    let responses = handles
        .into_iter()
        .map(|handle| handle.join().expect("bootstrap worker").expect("bootstrap"))
        .collect::<Vec<_>>();
    assert!(responses.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(responses[0]["committed"]["generation"], 0);
    assert!(responses[0]["pending"].is_null());

    let read = invoke(
        root.path(),
        json!({
            "operation": "readEnvironment",
            "environment": "test"
        }),
    )
    .expect("read converged state");
    assert_eq!(read, responses[0]);
}

#[test]
fn ecosystem_store_bootstrap_recovers_after_assembly_only_crash_point() {
    let root = TestRoot::new("assembly-only-crash");
    let store = CanonicalArtifactStore::create(root.path()).unwrap();
    let assembly = resolve_runtime_assembly(&[], &[], &[], &[]).unwrap();
    let assembly_path = store.write_runtime_assembly(&assembly).unwrap();
    let before = fs::read(&assembly_path).unwrap();

    let state_path = environment_state_path(root.path());
    assert!(!state_path.exists());
    let state = invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        }),
    )
    .expect("bootstrap after assembly-only crash point");

    assert_eq!(fs::read(assembly_path).unwrap(), before);
    assert_eq!(
        state["committed"]["assembly"],
        serde_json::to_value(runtime_assembly_ref(&assembly).unwrap()).unwrap()
    );
    assert!(state_path.is_file());
}

#[test]
fn ecosystem_store_bootstrap_preserves_conflicting_assembly_bytes() {
    let root = TestRoot::new("assembly-conflict");
    let assembly = resolve_runtime_assembly(&[], &[], &[], &[]).unwrap();
    let reference = runtime_assembly_ref(&assembly).unwrap();
    let assembly_path = root.path().join(
        RuntimeAssemblyRecordPath::new(&reference)
            .unwrap()
            .as_relative_path()
            .as_path(),
    );
    fs::create_dir_all(assembly_path.parent().unwrap()).unwrap();
    let conflict = b"{\"conflict\":true}";
    fs::write(&assembly_path, conflict).unwrap();

    let result = invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        }),
    );
    assert!(result.is_err(), "immutable conflict must fail closed");
    assert_eq!(fs::read(&assembly_path).unwrap(), conflict);
    assert!(
        !environment_state_path(root.path()).exists(),
        "conflicting assembly must not publish activation state"
    );
}

#[test]
fn ecosystem_store_bootstrap_does_not_treat_dangling_state_as_missing() {
    let root = TestRoot::new("dangling-state");
    invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        }),
    )
    .expect("seed canonical state bytes");
    let state_path = environment_state_path(root.path());
    let mut state: serde_json::Value =
        serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    state["committed"]["assembly"]["assemblyIdentity"] = json!(format!(
        "skiff-runtime-assembly-v1:sha256:{}",
        "f".repeat(64)
    ));
    let state_bytes = serde_json::to_vec(&state).unwrap();
    fs::write(&state_path, &state_bytes).unwrap();

    let empty = resolve_runtime_assembly(&[], &[], &[], &[]).unwrap();
    let empty_path = root.path().join(
        RuntimeAssemblyRecordPath::new(&runtime_assembly_ref(&empty).unwrap())
            .unwrap()
            .as_relative_path()
            .as_path(),
    );
    fs::remove_file(&empty_path).unwrap();

    let read_error = invoke(
        root.path(),
        json!({
            "operation": "readEnvironment",
            "environment": "test"
        }),
    )
    .expect_err("dangling state must fail typed reference validation");
    assert!(read_error.contains("runtime-assemblies"), "{read_error}");

    assert!(invoke(
        root.path(),
        json!({
            "operation": "ensureEnvironmentBootstrap",
            "environment": "test"
        })
    )
    .is_err());
    assert_eq!(fs::read(&state_path).unwrap(), state_bytes);
    assert!(
        !empty_path.exists(),
        "dangling state must not trigger bootstrap assembly publication"
    );
}

fn environment_state_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(
        EnvironmentActivationStatePath::new("test")
            .unwrap()
            .as_relative_path()
            .as_path(),
    )
}
