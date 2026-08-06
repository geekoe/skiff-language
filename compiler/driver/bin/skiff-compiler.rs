use std::path::PathBuf;

use skiff_artifact_identity::ReleasePointerPath;
use skiff_artifact_model::ServiceDeploymentRef;
use skiff_compiler::authoring::{
    build_authoring_object, project_runtime_assembly, seed_official_std_package, AuthoringObject,
};
use skiff_compiler::CompilerPlatformSources;
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};

const USAGE: &str = "usage:
  skiff-compiler package <build|publish> <root> --artifact-root <dir> [--profile <name>] [--json]
  skiff-compiler assembly <build|publish> --artifact-root <dir> --profile <name> [--root-deployment '<exact ServiceDeploymentRef JSON>']... [--json]
  skiff-compiler release set --artifact-root <dir> --profile <name> --deployment '<exact ServiceDeploymentRef JSON>' [--expected '<exact ReleasePointer JSON>'] [--json]
  skiff-compiler release <unset|get> --artifact-root <dir> --profile <name> --service <id> --version <v> [--expected '<exact ReleasePointer JSON>'] [--json]";

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
    if object == "std-seed" {
        // Internal tool action (absent from public help): canonical std seed
        // used by `skiff stack init` through the Node authoring library.
        return run_std_seed_action(args);
    }
    if object == "release" {
        return run_release_action(args);
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

fn run_std_seed_action(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut artifact_root = None;
    let mut platform_source_root = None;
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
            "--json" => json = true,
            _ => return Err(format!("unknown std-seed option {argument}").into()),
        }
    }
    let artifact_root = artifact_root.ok_or("--artifact-root is required")?;
    let platform_source_root = platform_source_root.ok_or("--platform-source-root is required")?;
    let platform_sources = CompilerPlatformSources::new(&platform_source_root)?;
    let receipt = seed_official_std_package(&platform_sources, &artifact_root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&receipt)?);
    } else {
        println!("{}", serde_json::to_string(&receipt)?);
    }
    Ok(())
}

fn run_release_action(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let action = args.next().ok_or(USAGE)?;
    match action.as_str() {
        "set" => run_release_set_action(args),
        "unset" => run_release_unset_action(args),
        "get" => run_release_get_action(args),
        _ => Err(format!("unknown release action {action}; expected set, unset, or get").into()),
    }
}

struct ReleaseOptions {
    artifact_root: PathBuf,
    profile: String,
    service_id: Option<String>,
    version: Option<String>,
    deployment_json: Option<String>,
    expected_json: Option<String>,
    json: bool,
}

fn parse_release_options(
    mut args: impl Iterator<Item = String>,
    action: &str,
) -> Result<ReleaseOptions, Box<dyn std::error::Error + Send + Sync>> {
    let mut artifact_root = None;
    let mut profile = None;
    let mut service_id = None;
    let mut version = None;
    let mut deployment_json = None;
    let mut expected_json = None;
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
            "--service" => {
                if service_id.is_some() {
                    return Err("--service was provided more than once".into());
                }
                service_id = Some(args.next().ok_or("--service requires an id")?);
            }
            "--version" => {
                if version.is_some() {
                    return Err("--version was provided more than once".into());
                }
                version = Some(args.next().ok_or("--version requires a value")?);
            }
            "--deployment" => {
                if deployment_json.is_some() {
                    return Err("--deployment was provided more than once".into());
                }
                deployment_json = Some(
                    args.next()
                        .ok_or("--deployment requires exact ServiceDeploymentRef JSON")?,
                );
            }
            "--expected" => {
                if expected_json.is_some() {
                    return Err("--expected was provided more than once".into());
                }
                expected_json = Some(
                    args.next()
                        .ok_or("--expected requires exact ReleasePointer JSON")?,
                );
            }
            "--json" => json = true,
            _ => return Err(format!("unknown release {action} option {argument}\n{USAGE}").into()),
        }
    }
    let artifact_root = artifact_root.ok_or("--artifact-root is required")?;
    let profile = profile.ok_or("--profile is required")?;
    Ok(ReleaseOptions {
        artifact_root,
        profile,
        service_id,
        version,
        deployment_json,
        expected_json,
        json,
    })
}

fn run_release_set_action(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let options = parse_release_options(&mut args, "set")?;
    let deployment_source = options
        .deployment_json
        .ok_or("release set requires --deployment '<exact ServiceDeploymentRef JSON>'")?;
    let deployment =
        serde_json::from_str::<ServiceDeploymentRef>(&deployment_source).map_err(|error| {
            format!("--deployment requires exact ServiceDeploymentRef JSON: {error}")
        })?;
    let candidate = ReleasePointer::new(&options.profile, deployment)?;
    let expected = match options.expected_json.as_deref() {
        Some(source) => Some(parse_expected_pointer(source, &candidate)?),
        None => None,
    };
    let store = CanonicalArtifactStore::create(&options.artifact_root)?;
    match expected.as_ref() {
        Some(expected) => store.compare_and_swap_release_pointer(Some(expected), &candidate)?,
        None => store.write_release_pointer(&candidate)?,
    }
    let pointer_path = ReleasePointerPath::new(
        &candidate.profile,
        &candidate.deployment.service_id,
        &candidate.deployment.contract_version,
    )?
    .as_str()
    .to_string();
    let receipt = serde_json::json!({
        "action": "set",
        "pointer": candidate,
        "pointerPath": pointer_path,
    });
    print_release_receipt(&receipt, options.json)
}

fn run_release_unset_action(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let options = parse_release_options(&mut args, "unset")?;
    let service_id = options
        .service_id
        .ok_or("release unset requires --service <id>")?;
    let version = options
        .version
        .ok_or("release unset requires --version <v>")?;
    let expected = match options.expected_json.as_deref() {
        Some(source) => Some(parse_expected_pointer_for_key(
            source,
            &options.profile,
            &service_id,
            &version,
        )?),
        None => None,
    };
    let store = CanonicalArtifactStore::create(&options.artifact_root)?;
    let removed =
        store.unset_release_pointer(&options.profile, &service_id, &version, expected.as_ref())?;
    let receipt = serde_json::json!({
        "action": "unset",
        "profile": options.profile,
        "serviceId": service_id,
        "version": version,
        "removedPointer": removed,
        "pointerPath": ReleasePointerPath::new(&options.profile, &service_id, &version)?.as_str(),
    });
    print_release_receipt(&receipt, options.json)
}

fn run_release_get_action(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let options = parse_release_options(&mut args, "get")?;
    let service_id = options
        .service_id
        .ok_or("release get requires --service <id>")?;
    let version = options
        .version
        .ok_or("release get requires --version <v>")?;
    let store = CanonicalArtifactStore::create(&options.artifact_root)?;
    let pointer = store.read_release_pointer(&options.profile, &service_id, &version)?;
    let receipt = serde_json::json!({
        "action": "get",
        "profile": options.profile,
        "serviceId": service_id,
        "version": version,
        "pointer": pointer,
        "pointerPath": ReleasePointerPath::new(&options.profile, &service_id, &version)?.as_str(),
    });
    print_release_receipt(&receipt, options.json)
}

fn parse_expected_pointer(
    source: &str,
    candidate: &ReleasePointer,
) -> Result<ReleasePointer, Box<dyn std::error::Error + Send + Sync>> {
    let pointer = serde_json::from_str::<ReleasePointer>(source)
        .map_err(|error| format!("--expected requires exact ReleasePointer JSON: {error}"))?;
    if pointer.profile != candidate.profile
        || pointer.deployment.service_id != candidate.deployment.service_id
        || pointer.deployment.contract_version != candidate.deployment.contract_version
    {
        return Err(
            "--expected release pointer must target the same profile, service, and version".into(),
        );
    }
    Ok(pointer)
}

fn parse_expected_pointer_for_key(
    source: &str,
    profile: &str,
    service_id: &str,
    version: &str,
) -> Result<ReleasePointer, Box<dyn std::error::Error + Send + Sync>> {
    let pointer = serde_json::from_str::<ReleasePointer>(source)
        .map_err(|error| format!("--expected requires exact ReleasePointer JSON: {error}"))?;
    if pointer.profile != profile
        || pointer.deployment.service_id != service_id
        || pointer.deployment.contract_version != version
    {
        return Err(
            "--expected release pointer must target the same profile, service, and version".into(),
        );
    }
    Ok(pointer)
}

fn print_release_receipt(
    receipt: &serde_json::Value,
    json: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if json {
        println!("{}", serde_json::to_string_pretty(receipt)?);
    } else {
        println!("{}", serde_json::to_string(receipt)?);
    }
    Ok(())
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
