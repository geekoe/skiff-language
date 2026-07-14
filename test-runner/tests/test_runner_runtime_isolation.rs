use std::{env, path::Path, process};

const INNER_MARKER: &str = "SKIFF_TEST_RUNNER_INNER";

fn main() {
    let inner_marker = env::var_os(INNER_MARKER);
    let worker_feature = cfg!(feature = "runtime-integration-worker");
    match (worker_feature, inner_marker.as_deref()) {
        (true, Some(value)) if value == "1" => {
            eprintln!(
                "[skiff-test] inner worker marker active; isolation wrapper will not recurse"
            );
            return;
        }
        (true, _) => {
            eprintln!(
                "error: runtime-integration-worker must run inside the isolated Cargo harness"
            );
            process::exit(1);
        }
        (false, Some(_)) => {
            eprintln!("error: {INNER_MARKER} is reserved for the isolated Cargo harness");
            process::exit(1);
        }
        (false, None) => {}
    }

    let skiff_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-runner manifest must be inside the Skiff workspace");
    let script = skiff_root.join("scripts/test-runner-runtime-isolation.mjs");
    let mut command = process::Command::new("node");
    command
        .arg(script)
        .args(env::args_os().skip(1))
        .current_dir(skiff_root);

    run_node_host(command);
}

#[cfg(unix)]
fn run_node_host(mut command: process::Command) {
    use std::os::unix::process::CommandExt;

    let error = command.exec();
    eprintln!("error: failed to exec isolated Node test host: {error}");
    process::exit(1);
}

#[cfg(not(unix))]
fn run_node_host(mut command: process::Command) {
    match command.status() {
        Ok(status) => process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("error: failed to run isolated Node test host: {error}");
            process::exit(1);
        }
    }
}
