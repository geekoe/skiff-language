use std::{env, path::PathBuf, process};

use skiff_test_runner::{run_skiff_tests_with_options, validate_activation_url, SkiffTestOptions};

const USAGE: &str = "usage: skiff-test-runner <input-file-or-dir> [--profile <name>] [--artifact-root <dir> --activation-url <url> --ingress-url <url>] [--environment <id> --expected-generation <n>] [--packages-dir <dir>]... [--package-test-concurrency <n>] [--live --allow-network --config <path>] [--deny-skips] [--require-tests]";

fn main() {
    if let Err(message) = run() {
        eprintln!("error: {message}");
        eprintln!("{USAGE}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let Some(args) = parse_args()? else {
        return Ok(());
    };
    execute(args)
}

struct CliArgs {
    input: PathBuf,
    profile: Option<String>,
    options: SkiffTestOptions,
    deny_skips: bool,
    require_tests: bool,
}

fn parse_args() -> Result<Option<CliArgs>, String> {
    let mut input = None;
    let mut profile = None;
    let mut artifact_root = None;
    let mut activation_url = None;
    let mut ingress_url = None;
    let mut environment = None;
    let mut expected_generation = None;
    let mut package_test_concurrency = None;
    let mut package_dirs = Vec::new();
    let mut config_path = None;
    let mut live = false;
    let mut allow_network = false;
    let mut deny_skips = false;
    let mut require_tests = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--profile" => set_once(&mut profile, next(&mut args, &arg)?, &arg)?,
            "--artifact-root" => set_once_path(&mut artifact_root, next(&mut args, &arg)?, &arg)?,
            "--activation-url" => {
                let value = next(&mut args, &arg)?;
                validate_activation_url(&value)?;
                set_once(&mut activation_url, value, &arg)?;
            }
            "--ingress-url" => set_once(&mut ingress_url, next(&mut args, &arg)?, &arg)?,
            "--environment" => set_once(&mut environment, next(&mut args, &arg)?, &arg)?,
            "--expected-generation" => {
                let value = next(&mut args, &arg)?;
                if expected_generation
                    .replace(parse_generation(&value, "--expected-generation")?)
                    .is_some()
                {
                    return Err("--expected-generation was provided more than once".to_string());
                }
            }
            "--packages-dir" => package_dirs.push(PathBuf::from(next(&mut args, &arg)?)),
            "--package-test-concurrency" => {
                let value = next(&mut args, &arg)?;
                let concurrency = value.parse::<usize>().map_err(|_| {
                    "--package-test-concurrency must be a positive integer".to_string()
                })?;
                if concurrency == 0 {
                    return Err("--package-test-concurrency must be a positive integer".to_string());
                }
                package_test_concurrency = Some(concurrency);
            }
            "--router-reload-url" => {
                let _ = next(&mut args, &arg)?;
                return Err(
                    "--router-reload-url is retired; use canonical --activation-url".to_string(),
                );
            }
            "--config" => set_once_path(&mut config_path, next(&mut args, &arg)?, &arg)?,
            "--live" => live = true,
            "--allow-network" => allow_network = true,
            "--deny-skips" => deny_skips = true,
            "--require-tests" => require_tests = true,
            value if value.starts_with('-') => return Err(format!("unknown option {value}")),
            value => set_once_path(&mut input, value.to_string(), "input")?,
        }
    }
    let input = input.ok_or_else(|| "missing input path".to_string())?;
    let artifact_root = artifact_root.or_else(|| env_path("SKIFF_TEST_ARTIFACT_ROOT"));
    let activation_url = activation_url.or_else(|| env::var("SKIFF_TEST_ACTIVATION_URL").ok());
    let ingress_url = ingress_url.or_else(|| env::var("SKIFF_TEST_INGRESS_URL").ok());
    let environment = environment
        .or_else(|| env::var("SKIFF_TEST_ENVIRONMENT").ok())
        .unwrap_or_else(|| "skiff-test".to_string());
    let expected_generation = match expected_generation {
        Some(value) => value,
        None => match env::var("SKIFF_TEST_EXPECTED_GENERATION") {
            Ok(value) => parse_generation(&value, "SKIFF_TEST_EXPECTED_GENERATION")?,
            Err(_) => 0,
        },
    };
    Ok(Some(CliArgs {
        input,
        profile,
        options: SkiffTestOptions {
            live,
            allow_network,
            config_path,
            package_dirs,
            artifact_root,
            activation_url,
            ingress_url,
            environment,
            expected_generation,
            package_test_concurrency,
        },
        deny_skips,
        require_tests,
    }))
}

fn execute(args: CliArgs) -> Result<(), String> {
    let summary = run_skiff_tests_with_options(&args.input, args.profile.as_deref(), &args.options)
        .map_err(|error| error.to_string())?;
    for result in &summary.results {
        let status = if result.skipped {
            "SKIP"
        } else if result.passed {
            "PASS"
        } else {
            "FAIL"
        };
        println!("{status} {}::{}", result.module_path, result.name);
        if let Some(message) = &result.message {
            println!("  {message}");
        }
    }
    if args.require_tests && summary.results.is_empty() {
        return Err("--require-tests requires at least one test".to_string());
    }
    if args.deny_skips && summary.skipped != 0 {
        return Err(format!(
            "--deny-skips forbids {} skipped test(s)",
            summary.skipped
        ));
    }
    if summary.failed != 0 {
        return Err(format!("{} test(s) failed", summary.failed));
    }
    println!(
        "test result: ok. {} passed; {} failed",
        summary.passed, summary.failed
    );
    Ok(())
}

fn next(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn set_once(target: &mut Option<String>, value: String, label: &str) -> Result<(), String> {
    if target.replace(value).is_some() {
        return Err(format!("{label} was provided more than once"));
    }
    Ok(())
}

fn set_once_path(target: &mut Option<PathBuf>, value: String, label: &str) -> Result<(), String> {
    if target.replace(PathBuf::from(value)).is_some() {
        return Err(format!("{label} was provided more than once"));
    }
    Ok(())
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn parse_generation(value: &str, label: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be an unsigned integer"))
}
