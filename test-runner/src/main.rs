use std::{
    env,
    path::{Path, PathBuf},
    process, thread,
};

use skiff_test_runner::{run_skiff_tests_with_options, SkiffTestOptions, SkiffTestSummary};

const TEST_RUNNER_STACK_SIZE: usize = 16 * 1024 * 1024;
const USAGE: &str = "usage: skiff-test-runner <input-file-or-dir> [--profile <name>] [--live --allow-network --config <config-path>] [--packages-dir <dir>]... [--service-artifact-root <dir>]... [--package-test-concurrency <n>]";

fn main() {
    match run(env::args().skip(1)) {
        Ok(()) => {}
        Err(CliError::Help) => {
            println!("{USAGE}");
        }
        Err(CliError::Message(message)) => {
            eprintln!("error: {message}");
            eprintln!("{USAGE}");
            process::exit(1);
        }
        Err(CliError::TestFailed) => process::exit(1),
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<(), CliError> {
    let args = parse_args(args)?;
    let summary = run_cli_skiff_tests(
        &args.input,
        args.profile.as_deref(),
        SkiffTestOptions {
            live: args.live,
            allow_network: args.allow_network,
            config_path: args.config_path,
            package_dirs: args.package_dirs,
            service_artifact_roots: args.service_artifact_roots,
            router_reload_url: None,
            package_test_concurrency: args.package_test_concurrency,
        },
    )?;
    print_summary(&summary);
    if summary.failed == 0 {
        Ok(())
    } else {
        Err(CliError::TestFailed)
    }
}

fn run_cli_skiff_tests(
    input: &Path,
    profile: Option<&str>,
    options: SkiffTestOptions,
) -> Result<SkiffTestSummary, CliError> {
    let input = input.to_path_buf();
    let profile = profile.map(str::to_string);
    thread::Builder::new()
        .name("skiff-cli-test-runner".to_string())
        .stack_size(TEST_RUNNER_STACK_SIZE)
        .spawn(move || run_skiff_tests_with_options(&input, profile.as_deref(), &options))
        .map_err(|error| CliError::message(format!("failed to start test runner: {error}")))?
        .join()
        .map_err(|_| CliError::message("test runner panicked"))?
        .map_err(|error| CliError::message(format!("test failed: {error}")))
}

fn print_summary(summary: &SkiffTestSummary) {
    for result in &summary.results {
        if result.skipped {
            println!("SKIP {}", result.name);
            if let Some(message) = &result.message {
                println!("  {message}");
            }
        } else if result.passed {
            println!("PASS {}", result.name);
        } else {
            println!("FAIL {}", result.name);
            if let Some(message) = &result.message {
                println!("  {message}");
            }
        }
    }
    if summary.failed == 0 {
        if summary.skipped == 0 {
            println!(
                "test result: ok. {} passed; {} failed",
                summary.passed, summary.failed
            );
        } else {
            println!(
                "test result: ok. {} passed; {} skipped; {} failed",
                summary.passed, summary.skipped, summary.failed
            );
        }
    } else if summary.skipped == 0 {
        println!(
            "test result: FAILED. {} passed; {} failed",
            summary.passed, summary.failed
        );
    } else {
        println!(
            "test result: FAILED. {} passed; {} skipped; {} failed",
            summary.passed, summary.skipped, summary.failed
        );
    }
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, CliError> {
    let mut input = None;
    let mut profile = None;
    let mut live = false;
    let mut allow_network = false;
    let mut config_path = None;
    let mut package_dirs = Vec::new();
    let mut service_artifact_roots = Vec::new();
    let mut package_test_concurrency = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(CliError::Help),
            "--live" => {
                if live {
                    return Err(CliError::message("--live was provided more than once"));
                }
                live = true;
            }
            "--allow-network" => {
                if allow_network {
                    return Err(CliError::message(
                        "--allow-network was provided more than once",
                    ));
                }
                allow_network = true;
            }
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--config requires a path"))?;
                if config_path.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message("--config was provided more than once"));
                }
            }
            "--profile" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--profile requires a name"))?;
                if value.is_empty() {
                    return Err(CliError::message("--profile cannot be empty"));
                }
                if profile.replace(value).is_some() {
                    return Err(CliError::message("--profile was provided more than once"));
                }
            }
            "--packages-dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--packages-dir requires a path"))?;
                package_dirs.push(PathBuf::from(value));
            }
            _ if arg.starts_with("--packages-dir=") => {
                package_dirs.push(PathBuf::from(&arg["--packages-dir=".len()..]));
            }
            "--service-artifact-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--service-artifact-root requires a path"))?;
                service_artifact_roots.push(PathBuf::from(value));
            }
            _ if arg.starts_with("--service-artifact-root=") => {
                service_artifact_roots
                    .push(PathBuf::from(&arg["--service-artifact-root=".len()..]));
            }
            "--package-test-concurrency" => {
                let value = args.next().ok_or_else(|| {
                    CliError::message("--package-test-concurrency requires a positive integer")
                })?;
                let value = parse_positive_usize(&value, "--package-test-concurrency")?;
                if package_test_concurrency.replace(value).is_some() {
                    return Err(CliError::message(
                        "--package-test-concurrency was provided more than once",
                    ));
                }
            }
            _ if arg.starts_with("--package-test-concurrency=") => {
                let value = parse_positive_usize(
                    &arg["--package-test-concurrency=".len()..],
                    "--package-test-concurrency",
                )?;
                if package_test_concurrency.replace(value).is_some() {
                    return Err(CliError::message(
                        "--package-test-concurrency was provided more than once",
                    ));
                }
            }
            _ if arg.starts_with('-') => {
                return Err(CliError::message(format!("unknown option {arg}")));
            }
            _ => {
                if input.replace(PathBuf::from(arg)).is_some() {
                    return Err(CliError::message("multiple input paths provided"));
                }
            }
        }
    }

    let input = input.ok_or_else(|| CliError::message("missing input path"))?;
    Ok(CliArgs {
        input,
        profile,
        live,
        allow_network,
        config_path,
        package_dirs,
        service_artifact_roots,
        package_test_concurrency,
    })
}

fn parse_positive_usize(value: &str, source: &str) -> Result<usize, CliError> {
    let parsed = value.parse::<usize>().map_err(|_| {
        CliError::message(format!("{source} must be a positive integer, got {value}"))
    })?;
    if parsed == 0 {
        return Err(CliError::message(format!(
            "{source} must be a positive integer, got {value}"
        )));
    }
    Ok(parsed)
}

struct CliArgs {
    input: PathBuf,
    profile: Option<String>,
    live: bool,
    allow_network: bool,
    config_path: Option<PathBuf>,
    package_dirs: Vec<PathBuf>,
    service_artifact_roots: Vec<PathBuf>,
    package_test_concurrency: Option<usize>,
}

enum CliError {
    Help,
    Message(String),
    TestFailed,
}

impl CliError {
    fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
