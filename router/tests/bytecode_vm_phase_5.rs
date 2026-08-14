use std::{path::Path, process::Command};

const ROUTER_BIN_ENV: &str = "SKIFF_BYTECODE_VM_PHASE5_ROUTER_BIN";

#[test]
fn phase_5_router_full_chain_vcp() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Router crate has repository parent");
    let script = repository.join("scripts/lib/bytecode-vm-phase-5-router-harness.mjs");
    assert!(script.is_file(), "Phase 5 Router harness is missing");

    let output = Command::new(std::env::var_os("NODE").unwrap_or_else(|| "node".into()))
        .arg(&script)
        .current_dir(repository)
        .env(ROUTER_BIN_ENV, env!("CARGO_BIN_EXE_skiff-router"))
        .output()
        .expect("start Phase 5 production Router harness");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Phase 5 production Router harness failed at a real process boundary\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains(r#""verdict":"PASS""#),
        "Phase 5 Router harness omitted its observable PASS verdict:\n{stdout}"
    );
}
