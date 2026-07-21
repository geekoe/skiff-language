use std::{env, path::PathBuf, process};

use skiff_test_runner::{
    run_skiff_tests_with_options, validate_activation_url, validate_ingress_url, SkiffTestOptions,
};

const USAGE: &str = "usage: skiff-test-runner <input-file-or-dir> --artifact-root <dir> [--base-assembly <identity>] [--live --activation-url <url> --ingress-url <url> --environment <id> --expected-generation <n>] [--deny-skips] [--require-tests]";

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
    options: SkiffTestOptions,
    deny_skips: bool,
    require_tests: bool,
}

#[derive(Default)]
struct RawCliArgs {
    input: Option<PathBuf>,
    artifact_root: Option<PathBuf>,
    base_assembly: Option<String>,
    activation_url: Option<String>,
    ingress_url: Option<String>,
    environment: Option<String>,
    expected_generation: Option<u64>,
    live: bool,
    deny_skips: bool,
    require_tests: bool,
}

fn parse_args() -> Result<Option<CliArgs>, String> {
    let mut parsed = RawCliArgs::default();
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--artifact-root" => {
                set_once_path(&mut parsed.artifact_root, next(&mut args, &arg)?, &arg)?
            }
            "--base-assembly" => set_once(&mut parsed.base_assembly, next(&mut args, &arg)?, &arg)?,
            "--activation-url" => {
                let value = next(&mut args, &arg)?;
                validate_activation_url(&value)?;
                set_once(&mut parsed.activation_url, value, &arg)?;
            }
            "--ingress-url" => {
                let value = next(&mut args, &arg)?;
                validate_ingress_url(&value)?;
                set_once(&mut parsed.ingress_url, value, &arg)?;
            }
            "--environment" => {
                let value = next(&mut args, &arg)?;
                validate_environment(&value)?;
                set_once(&mut parsed.environment, value, &arg)?;
            }
            "--expected-generation" => {
                let value = next(&mut args, &arg)?;
                if parsed
                    .expected_generation
                    .replace(parse_generation(&value, "--expected-generation")?)
                    .is_some()
                {
                    return Err("--expected-generation was provided more than once".to_string());
                }
            }
            "--live" => set_flag(&mut parsed.live, &arg)?,
            "--deny-skips" => set_flag(&mut parsed.deny_skips, &arg)?,
            "--require-tests" => set_flag(&mut parsed.require_tests, &arg)?,
            value if value.starts_with('-') => return Err(format!("unknown option {value}")),
            value => set_once_path(&mut parsed.input, value.to_string(), "input")?,
        }
    }
    finish_args(parsed).map(Some)
}

fn finish_args(parsed: RawCliArgs) -> Result<CliArgs, String> {
    let RawCliArgs {
        input,
        artifact_root,
        base_assembly,
        activation_url,
        ingress_url,
        environment,
        expected_generation,
        live,
        deny_skips,
        require_tests,
    } = parsed;
    let input = input.ok_or_else(|| "missing input path".to_string())?;
    let artifact_root = artifact_root.ok_or_else(|| "missing --artifact-root".to_string())?;
    if live
        && (activation_url.is_none()
            || ingress_url.is_none()
            || environment.is_none()
            || expected_generation.is_none())
    {
        return Err(
            "--live requires --activation-url, --ingress-url, --environment and --expected-generation"
                .to_string(),
        );
    }
    if !live
        && (activation_url.is_some()
            || ingress_url.is_some()
            || environment.is_some()
            || expected_generation.is_some())
    {
        return Err(
            "non-live targets are supplied only by the isolated runtime harness".to_string(),
        );
    }
    let runtime_artifact_root = (!live)
        .then(|| env_path("SKIFF_TEST_RUNTIME_ARTIFACT_ROOT"))
        .flatten();
    let activation_url = if live {
        activation_url
    } else {
        env::var("SKIFF_TEST_ACTIVATION_URL").ok()
    };
    let ingress_url = if live {
        ingress_url
    } else {
        env::var("SKIFF_TEST_INGRESS_URL").ok()
    };
    if let Some(value) = activation_url.as_deref() {
        validate_activation_url(value)?;
    }
    if let Some(value) = ingress_url.as_deref() {
        validate_ingress_url(value)?;
    }
    let environment = if live {
        environment.expect("live environment was checked")
    } else {
        env::var("SKIFF_TEST_ENVIRONMENT").unwrap_or_else(|_| "skiff-test".to_string())
    };
    validate_environment(&environment)?;
    let expected_generation = if live {
        expected_generation.expect("live generation was checked")
    } else {
        match env::var("SKIFF_TEST_EXPECTED_GENERATION") {
            Ok(value) => parse_generation(&value, "SKIFF_TEST_EXPECTED_GENERATION")?,
            Err(_) => 0,
        }
    };
    Ok(CliArgs {
        input,
        options: SkiffTestOptions {
            live,
            artifact_root: Some(artifact_root),
            runtime_artifact_root,
            base_assembly,
            activation_url,
            ingress_url,
            environment,
            expected_generation,
        },
        deny_skips,
        require_tests,
    })
}

fn execute(args: CliArgs) -> Result<(), String> {
    let summary = run_skiff_tests_with_options(&args.input, &args.options)
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

fn set_flag(target: &mut bool, label: &str) -> Result<(), String> {
    if *target {
        return Err(format!("{label} was provided more than once"));
    }
    *target = true;
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

fn validate_environment(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("environment must be a canonical ASCII token".to_string());
    }
    Ok(())
}
