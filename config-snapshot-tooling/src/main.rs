use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::json;
use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_config_snapshot_tooling::{
    produce_runtime_config_snapshot, ConfigSnapshotProductionInput, ServiceConfigSource,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("config snapshot production failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse(env::args().skip(1))?;
    let expected_deployments = arguments
        .sources
        .iter()
        .map(|source| source.deployment.clone())
        .collect::<BTreeSet<_>>();
    let deployments = discover_exact_deployments(&arguments.artifact_root, &expected_deployments)?;
    let package_refs = required_package_refs(&deployments);
    let packages = discover_exact_packages(&arguments.artifact_root, &package_refs)?;
    let receipt = produce_runtime_config_snapshot(
        ConfigSnapshotProductionInput {
            profile: arguments.profile,
            deployments,
            package_artifacts: packages,
            sources: arguments.sources,
        },
        &arguments.artifact_root,
    )?;
    let output = json!({
        "runtimeConfigSnapshotReceipt": receipt
    });
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

#[derive(Debug)]
struct Arguments {
    artifact_root: PathBuf,
    profile: String,
    sources: Vec<ServiceConfigSource>,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut artifact_root = None;
        let mut profile = None;
        let mut sources = Vec::new();
        let mut arguments = arguments.peekable();
        while let Some(argument) = arguments.next() {
            let value = arguments
                .next()
                .ok_or_else(|| format!("{argument} requires a value"))?;
            match argument.as_str() {
                "--artifact-root" if artifact_root.is_none() => {
                    artifact_root = Some(PathBuf::from(value));
                }
                "--profile" if profile.is_none() => profile = Some(value),
                "--source" => {
                    let source = serde_json::from_str::<ServiceConfigSource>(&value)
                        .map_err(|error| format!("--source must be strict JSON: {error}"))?;
                    sources.push(source);
                }
                "--artifact-root" | "--profile" => {
                    return Err(format!("{argument} was provided more than once"));
                }
                _ => return Err(format!("unknown option {argument}")),
            }
        }
        let artifact_root = artifact_root.ok_or("--artifact-root is required")?;
        if !artifact_root.is_absolute() {
            return Err("--artifact-root must be absolute".to_string());
        }
        Ok(Self {
            artifact_root,
            profile: profile.ok_or("--profile is required")?,
            sources,
        })
    }
}

fn discover_exact_deployments(
    artifact_root: &Path,
    expected: &BTreeSet<ServiceDeploymentRef>,
) -> Result<BTreeMap<ServiceDeploymentRef, ServiceDeployment>, Box<dyn std::error::Error>> {
    if expected.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut files = Vec::new();
    collect_deployment_records(
        &artifact_root.join("records").join("service-deployments"),
        &mut files,
    )?;
    let mut found = BTreeMap::new();
    for path in files {
        let bytes = fs::read(&path)?;
        let deployment = match serde_json::from_slice::<ServiceDeployment>(&bytes) {
            Ok(deployment) => deployment,
            Err(_) => continue,
        };
        let reference = deployment_reference(&deployment);
        if !expected.contains(&reference) {
            continue;
        }
        if found.insert(reference, deployment).is_some() {
            return Err(format!(
                "duplicate exact ServiceDeployment record at {}",
                path.display()
            )
            .into());
        }
    }
    let missing = expected
        .iter()
        .filter(|reference| !found.contains_key(reference))
        .map(|reference| reference.service_id.as_str())
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "artifact root is missing exact ServiceDeployment record(s): {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    Ok(found)
}

fn deployment_reference(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn required_package_refs(
    deployments: &BTreeMap<ServiceDeploymentRef, ServiceDeployment>,
) -> Vec<PackageArtifactRef> {
    let mut references = Vec::new();
    for deployment in deployments.values() {
        references.push(deployment.implementation.clone());
        references.extend(
            deployment
                .package_bindings
                .iter()
                .map(|binding| binding.package.clone()),
        );
    }
    references
}

fn discover_exact_packages(
    artifact_root: &Path,
    expected: &[PackageArtifactRef],
) -> Result<BTreeMap<PackageArtifactRef, PackageArtifact>, Box<dyn std::error::Error>> {
    let expected_by_build = expected
        .iter()
        .map(|reference| {
            (
                reference.package_build_id.as_str().to_string(),
                reference.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if expected_by_build.len() != expected.len() {
        return Err(
            "deployment package closure contains duplicate PackageBuildId references".into(),
        );
    }
    if expected.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut files = Vec::new();
    collect_package_records(
        &artifact_root.join("records").join("package-artifacts"),
        &mut files,
    )?;
    let mut found = BTreeMap::new();
    for path in files {
        let bytes = fs::read(&path)?;
        let raw = match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let Some(build) = raw
            .get("packageBuildId")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(expected_ref) = expected_by_build.get(build) else {
            continue;
        };
        let artifact = serde_json::from_value::<PackageArtifact>(raw)?;
        let actual_ref = PackageArtifactRef {
            package_id: artifact.package_id.clone(),
            package_version: artifact.package_version.clone(),
            package_build_id: artifact.package_build_id.clone(),
            package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
        };
        if &actual_ref != expected_ref {
            return Err(format!(
                "package record {} does not match deployment reference for {}",
                path.display(),
                expected_ref.package_id
            )
            .into());
        }
        if found.insert(actual_ref, artifact).is_some() {
            return Err(format!(
                "duplicate exact PackageArtifact record at {}",
                path.display()
            )
            .into());
        }
    }
    let missing = expected
        .iter()
        .filter(|reference| !found.contains_key(reference))
        .map(|reference| reference.package_id.as_str())
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "artifact root is missing exact PackageArtifact record(s): {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    Ok(found)
}

fn collect_deployment_records(
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(format!(
                "service deployment record tree contains symlink {}",
                entry.path().display()
            )
            .into());
        }
        if file_type.is_dir() {
            collect_deployment_records(&entry.path(), output)?;
        } else if file_type.is_file() && entry.file_name().to_string_lossy().ends_with(".json") {
            output.push(entry.path());
        }
    }
    Ok(())
}

fn collect_package_records(
    directory: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(format!(
                "package artifact record tree contains symlink {}",
                entry.path().display()
            )
            .into());
        }
        if file_type.is_dir() {
            collect_package_records(&entry.path(), output)?;
        } else if file_type.is_file() && entry.file_name() == "package.json" {
            output.push(entry.path());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
