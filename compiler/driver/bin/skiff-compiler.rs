use std::path::PathBuf;

use skiff_artifact_model::ServiceDeploymentRef;
use skiff_compiler::authoring::{
    build_authoring_object, project_runtime_assembly, AuthoringObject,
};
use skiff_compiler::CompilerPlatformSources;

const USAGE: &str = "usage:
  skiff-compiler package <build|publish> <root> --artifact-root <dir> [--profile <name>] [--json]
  skiff-compiler assembly <build|publish> --artifact-root <dir> --profile <name> [--root-deployment '<exact ServiceDeploymentRef JSON>']... [--json]";

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
    match object {
        AuthoringObject::Package => run_package_action(args, publish_pointer),
        AuthoringObject::Assembly => run_assembly_action(args, publish_pointer),
    }
}

fn run_package_action(
    mut args: impl Iterator<Item = String>,
    publish_pointer: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = PathBuf::from(args.next().ok_or(USAGE)?);
    let mut artifact_root = None;
    let mut platform_source_root = None;
    let mut profile = None;
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
            "--profile" => {
                if profile.is_some() {
                    return Err("--profile was provided more than once".into());
                }
                profile = Some(args.next().ok_or("--profile requires a name")?);
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
        AuthoringObject::Package,
        &root,
        &artifact_root,
        profile.as_deref().unwrap_or("dev"),
        publish_pointer,
    )?;
    print_receipt(&receipt, json)
}

fn run_assembly_action(
    mut args: impl Iterator<Item = String>,
    publish_pointer: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut artifact_root = None;
    let mut profile = None;
    let mut root_deployments = Vec::new();
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
            "--profile" => {
                if profile.is_some() {
                    return Err("--profile was provided more than once".into());
                }
                profile = Some(args.next().ok_or("--profile requires a name")?);
            }
            "--root-deployment" => {
                let source = args
                    .next()
                    .ok_or("--root-deployment requires exact ServiceDeploymentRef JSON")?;
                let reference =
                    serde_json::from_str::<ServiceDeploymentRef>(&source).map_err(|error| {
                        format!(
                            "--root-deployment requires exact ServiceDeploymentRef JSON: {error}"
                        )
                    })?;
                root_deployments.push(reference);
            }
            "--json" => json = true,
            _ => return Err(format!("unknown assembly option {argument}\n{USAGE}").into()),
        }
    }
    let artifact_root = artifact_root.ok_or("--artifact-root is required")?;
    let profile = profile.ok_or("--profile is required for assembly projection")?;
    let receipt =
        project_runtime_assembly(&artifact_root, &profile, &root_deployments, publish_pointer)?;
    print_receipt(&receipt, json)
}

fn print_receipt(
    receipt: &serde_json::Value,
    json: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if json {
        println!("{}", serde_json::to_string_pretty(receipt)?);
    } else {
        println!("{}", render_authoring_receipt(receipt)?);
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
mod tests;
