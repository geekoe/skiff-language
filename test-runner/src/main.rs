use std::{
    env,
    path::{Path, PathBuf},
    process, thread,
};

use skiff_test_runner::{
    run_skiff_tests_with_options, validate_runtime_reload_url, SkiffTestOptions, SkiffTestSummary,
};

const TEST_RUNNER_STACK_SIZE: usize = 16 * 1024 * 1024;
const USAGE: &str = "usage: skiff-test-runner <input-file-or-dir> [--profile <name>] [--live --allow-network --config <config-path>] [--router-reload-url <url>] [--artifact-root <dir>] [--deny-skips] [--require-tests] [--packages-dir <dir>]... [--service-artifact-root <dir>]... [--package-test-concurrency <n>]";

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
    validate_explicit_runtime_inputs(&args)?;
    let summary = run_cli_skiff_tests(
        &args.input,
        args.profile.as_deref(),
        SkiffTestOptions {
            live: args.live,
            allow_network: args.allow_network,
            config_path: args.config_path,
            package_dirs: args.package_dirs,
            service_artifact_roots: args.service_artifact_roots,
            router_reload_url: args.router_reload_url,
            artifact_root: args.artifact_root,
            package_test_concurrency: args.package_test_concurrency,
        },
    )?;
    let policy_failures = summary_policy_failures(&summary, args.deny_skips, args.require_tests);
    let succeeded = summary.failed == 0 && policy_failures.is_empty();
    print_summary(&summary, succeeded, &policy_failures);
    if succeeded {
        Ok(())
    } else {
        Err(CliError::TestFailed)
    }
}

fn validate_explicit_runtime_inputs(args: &CliArgs) -> Result<(), CliError> {
    if let Some(artifact_root) = &args.artifact_root {
        if !artifact_root.is_dir() {
            return Err(CliError::message(format!(
                "--artifact-root must be an existing directory: {}",
                artifact_root.display()
            )));
        }
    }
    if let Some(reload_url) = &args.router_reload_url {
        validate_runtime_reload_url(reload_url).map_err(CliError::message)?;
    }
    Ok(())
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

fn print_summary(summary: &SkiffTestSummary, succeeded: bool, policy_failures: &[String]) {
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
    if succeeded {
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
    for failure in policy_failures {
        println!("strict policy failure: {failure}");
    }
}

fn summary_policy_failures(
    summary: &SkiffTestSummary,
    deny_skips: bool,
    require_tests: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if deny_skips && summary.skipped > 0 {
        failures.push(format!(
            "--deny-skips forbids {} skipped test(s); see the SKIP reasons above",
            summary.skipped
        ));
    }
    if require_tests && summary.passed + summary.skipped + summary.failed == 0 {
        failures.push("--require-tests requires at least one discovered test result".to_string());
    }
    failures
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, CliError> {
    let mut input = None;
    let mut profile = None;
    let mut live = false;
    let mut allow_network = false;
    let mut config_path = None;
    let mut package_dirs = Vec::new();
    let mut service_artifact_roots = Vec::new();
    let mut router_reload_url = None;
    let mut artifact_root = None;
    let mut deny_skips = false;
    let mut require_tests = false;
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
            "--deny-skips" => {
                if deny_skips {
                    return Err(CliError::message(
                        "--deny-skips was provided more than once",
                    ));
                }
                deny_skips = true;
            }
            "--require-tests" => {
                if require_tests {
                    return Err(CliError::message(
                        "--require-tests was provided more than once",
                    ));
                }
                require_tests = true;
            }
            "--config" => {
                let value = required_next_value(&mut args, "--config", "a path")?;
                if config_path.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message("--config was provided more than once"));
                }
            }
            _ if arg.starts_with("--config=") => {
                let value = required_inline_value(&arg, "--config")?;
                if config_path.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message("--config was provided more than once"));
                }
            }
            "--profile" => {
                let value = required_next_value(&mut args, "--profile", "a name")?;
                if value.is_empty() {
                    return Err(CliError::message("--profile cannot be empty"));
                }
                if profile.replace(value).is_some() {
                    return Err(CliError::message("--profile was provided more than once"));
                }
            }
            _ if arg.starts_with("--profile=") => {
                let value = required_inline_value(&arg, "--profile")?;
                if profile.replace(value.to_string()).is_some() {
                    return Err(CliError::message("--profile was provided more than once"));
                }
            }
            "--router-reload-url" => {
                let value = required_next_value(&mut args, "--router-reload-url", "a URL")?;
                if value.is_empty() {
                    return Err(CliError::message("--router-reload-url cannot be empty"));
                }
                if router_reload_url.replace(value).is_some() {
                    return Err(CliError::message(
                        "--router-reload-url was provided more than once",
                    ));
                }
            }
            _ if arg.starts_with("--router-reload-url=") => {
                let value = required_inline_value(&arg, "--router-reload-url")?;
                if router_reload_url.replace(value.to_string()).is_some() {
                    return Err(CliError::message(
                        "--router-reload-url was provided more than once",
                    ));
                }
            }
            "--artifact-root" => {
                let value = required_next_value(&mut args, "--artifact-root", "a path")?;
                if value.is_empty() {
                    return Err(CliError::message("--artifact-root cannot be empty"));
                }
                if artifact_root.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message(
                        "--artifact-root was provided more than once",
                    ));
                }
            }
            _ if arg.starts_with("--artifact-root=") => {
                let value = required_inline_value(&arg, "--artifact-root")?;
                if artifact_root.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message(
                        "--artifact-root was provided more than once",
                    ));
                }
            }
            "--packages-dir" => {
                let value = required_next_value(&mut args, "--packages-dir", "a path")?;
                package_dirs.push(PathBuf::from(value));
            }
            _ if arg.starts_with("--packages-dir=") => {
                package_dirs.push(PathBuf::from(&arg["--packages-dir=".len()..]));
            }
            "--service-artifact-root" => {
                let value = required_next_value(&mut args, "--service-artifact-root", "a path")?;
                service_artifact_roots.push(PathBuf::from(value));
            }
            _ if arg.starts_with("--service-artifact-root=") => {
                service_artifact_roots
                    .push(PathBuf::from(&arg["--service-artifact-root=".len()..]));
            }
            "--package-test-concurrency" => {
                let value = required_next_value(
                    &mut args,
                    "--package-test-concurrency",
                    "a positive integer",
                )?;
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
        router_reload_url,
        artifact_root,
        deny_skips,
        require_tests,
        package_test_concurrency,
    })
}

fn required_inline_value<'a>(arg: &'a str, option: &str) -> Result<&'a str, CliError> {
    let value = &arg[option.len() + 1..];
    if value.is_empty() {
        return Err(CliError::message(format!("{option} requires a value")));
    }
    Ok(value)
}

fn required_next_value(
    args: &mut impl Iterator<Item = String>,
    option: &str,
    expected: &str,
) -> Result<String, CliError> {
    let value = args
        .next()
        .ok_or_else(|| CliError::message(format!("{option} requires {expected}")))?;
    if value.is_empty() || value.starts_with('-') {
        return Err(CliError::message(format!("{option} requires {expected}")));
    }
    Ok(value)
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

#[derive(Debug)]
struct CliArgs {
    input: PathBuf,
    profile: Option<String>,
    live: bool,
    allow_network: bool,
    config_path: Option<PathBuf>,
    package_dirs: Vec<PathBuf>,
    service_artifact_roots: Vec<PathBuf>,
    router_reload_url: Option<String>,
    artifact_root: Option<PathBuf>,
    deny_skips: bool,
    require_tests: bool,
    package_test_concurrency: Option<usize>,
}

#[derive(Debug)]
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

#[cfg(test)]
mod main_tests;
