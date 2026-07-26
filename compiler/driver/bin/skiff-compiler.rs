use std::path::PathBuf;

use skiff_compiler::authoring::{build_authoring_object, AuthoringObject};
use skiff_compiler::CompilerPlatformSources;

const USAGE: &str = "usage: skiff-compiler <package|assembly> <build|publish> <root> --artifact-root <dir> [--environment <name>] [--json]";

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_with_args(std::env::args().skip(1))
}

fn run_with_args(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let object = args.next().ok_or(USAGE)?;
    if object == "-h" || object == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    let object = AuthoringObject::parse(&object)?;
    let action = args.next().ok_or(USAGE)?;
    let publish_pointer = match action.as_str() {
        "build" => false,
        "publish" => true,
        _ => {
            return Err(
                format!("unknown authoring action {action}; expected build or publish").into(),
            )
        }
    };
    let root = PathBuf::from(args.next().ok_or(USAGE)?);
    let mut artifact_root = None;
    let mut platform_source_root = None;
    let mut environment = None;
    let mut json = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--artifact-root" => {
                if artifact_root.is_some() {
                    return Err("--artifact-root was provided more than once".into());
                }
                artifact_root = Some(PathBuf::from(
                    args.next().ok_or("--artifact-root requires a path")?,
                ));
            }
            "--platform-source-root" => {
                if platform_source_root.is_some() {
                    return Err("--platform-source-root was provided more than once".into());
                }
                platform_source_root = Some(PathBuf::from(
                    args.next()
                        .ok_or("--platform-source-root requires a path")?,
                ));
            }
            "--environment" => {
                if environment.is_some() {
                    return Err("--environment was provided more than once".into());
                }
                environment = Some(args.next().ok_or("--environment requires a name")?);
            }
            "--json" => json = true,
            _ => return Err(format!("unknown option {argument}\n{USAGE}").into()),
        }
    }
    let artifact_root = artifact_root.ok_or("--artifact-root is required")?;
    let platform_source_root = platform_source_root.ok_or("--platform-source-root is required")?;
    let platform_sources = CompilerPlatformSources::new(&platform_source_root)?;
    let receipt = build_authoring_object(
        &platform_sources,
        object,
        &root,
        &artifact_root,
        environment.as_deref().unwrap_or("dev"),
        publish_pointer,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&receipt)?);
    } else {
        println!("{}", render_authoring_receipt(&receipt)?);
    }
    Ok(())
}

fn render_authoring_receipt(
    receipt: &serde_json::Value,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let Some(api) = receipt.get("serviceApiReceipt") else {
        return Ok(serde_json::to_string(receipt)?);
    };
    let service = api
        .get("serviceId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<package only>");
    let functions = api
        .pointer("/projection/functions")
        .and_then(serde_json::Value::as_array)
        .ok_or("service API receipt is missing projection.functions")?;
    let available = functions
        .iter()
        .filter(|function| {
            function.get("status").and_then(serde_json::Value::as_str) == Some("available")
                && function
                    .get("serviceOperationId")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
        })
        .count();
    let package_only = functions.len() - available;
    let mut lines = vec![
        format!("Service API for {service}"),
        format!("Available: {available}"),
        format!("Package-only: {package_only}"),
    ];
    for function in functions {
        let path = function
            .get("publicPath")
            .and_then(serde_json::Value::as_str)
            .ok_or("service API function is missing publicPath")?;
        match function.get("status").and_then(serde_json::Value::as_str) {
            Some("available")
                if function
                    .get("serviceOperationId")
                    .and_then(serde_json::Value::as_str)
                    .is_some() =>
            {
                lines.push(format!("  available {path}"))
            }
            Some("available") => lines.push(format!("  package-only {path}")),
            Some("unavailable") => {
                lines.push(format!("  package-only {path}"));
                let reasons = function
                    .get("reasons")
                    .and_then(serde_json::Value::as_array)
                    .ok_or("unavailable service API function is missing reasons")?;
                for reason in reasons {
                    lines.push(format!("    - {}", serde_json::to_string(reason)?));
                }
            }
            _ => return Err("service API function has an unknown status".into()),
        }
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{render_authoring_receipt, run_with_args, USAGE};

    #[test]
    fn internal_actions_are_absent_from_public_help() {
        assert!(!USAGE.contains("platform-source"));
        for object in ["package", "assembly"] {
            assert!(USAGE.contains(object));
        }
    }

    #[test]
    fn authoring_actions_require_exactly_one_platform_source_root() {
        for object in ["package", "assembly"] {
            let missing = run_error(&[
                object,
                "build",
                "/missing-authoring-root",
                "--artifact-root",
                "/tmp/skiff-artifacts",
            ]);
            assert_eq!(missing, "--platform-source-root is required");

            let duplicate = run_error(&[
                object,
                "build",
                "/missing-authoring-root",
                "--artifact-root",
                "/tmp/skiff-artifacts",
                "--platform-source-root",
                "/missing-platform-root-a",
                "--platform-source-root",
                "/missing-platform-root-b",
            ]);
            assert_eq!(
                duplicate,
                "--platform-source-root was provided more than once"
            );
        }
    }

    #[test]
    fn authoring_actions_reject_relative_or_unreadable_platform_source_roots_first() {
        let relative = run_error(&[
            "package",
            "build",
            "/missing-authoring-root",
            "--artifact-root",
            "/tmp/skiff-artifacts",
            "--platform-source-root",
            "relative/platform-root",
        ]);
        assert!(relative.contains("must be absolute"), "{relative}");

        let unreadable = run_error(&[
            "package",
            "build",
            "/missing-authoring-root",
            "--artifact-root",
            "/tmp/skiff-artifacts",
            "--platform-source-root",
            "/missing-skiff-platform-root",
        ]);
        assert!(
            unreadable.contains("compiler platform source"),
            "{unreadable}"
        );
        assert!(!unreadable.contains("contract.yml"), "{unreadable}");
    }

    #[test]
    fn human_service_api_output_never_hides_package_only_functions() {
        let rendered = render_authoring_receipt(&json!({
            "serviceApiReceipt": {
                "serviceId": "example.registry",
                "serviceProtocolIdentity": "protocol",
                "projection": {
                    "functions": [
                        {
                            "publicPath": "read",
                            "callableId": "read-id",
                            "status": "available",
                            "serviceOperationId": "read-operation"
                        },
                        {
                            "publicPath": "inspect",
                            "callableId": "inspect-id",
                            "status": "available"
                        },
                        {
                            "publicPath": "unsafeWrite",
                            "callableId": "write-id",
                            "status": "unavailable",
                            "reasons": ["writesCallerReachable", "returnsCallerAlias"]
                        }
                    ]
                }
            }
        }))
        .unwrap();
        assert_eq!(
            rendered,
            "Service API for example.registry\nAvailable: 1\nPackage-only: 2\n  available read\n  package-only inspect\n  package-only unsafeWrite\n    - \"writesCallerReachable\"\n    - \"returnsCallerAlias\""
        );
    }

    #[test]
    fn human_service_api_output_is_explicit_for_zero_api() {
        let rendered = render_authoring_receipt(&json!({
            "serviceApiReceipt": {
                "serviceId": "example.empty",
                "serviceProtocolIdentity": "protocol",
                "projection": { "functions": [] }
            }
        }))
        .unwrap();
        assert_eq!(
            rendered,
            "Service API for example.empty\nAvailable: 0\nPackage-only: 0"
        );
    }

    fn run_error(args: &[&str]) -> String {
        run_with_args(args.iter().map(|argument| (*argument).to_owned()))
            .unwrap_err()
            .to_string()
    }
}
