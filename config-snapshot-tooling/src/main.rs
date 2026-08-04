use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Component, Path, PathBuf},
};

use serde::Serialize;
use serde_json::json;
use skiff_artifact_model::{PackageArtifact, PackageArtifactRef, RuntimeAssembly};
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
    let assembly_path = resolve_record(&arguments.artifact_root, &arguments.assembly_record)?;
    let assembly = read_json::<RuntimeAssembly>(&assembly_path)?;
    let packages = discover_exact_packages(&arguments.artifact_root, &assembly)?;
    let receipt = produce_runtime_config_snapshot(
        ConfigSnapshotProductionInput {
            profile: arguments.profile,
            assembly,
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
    assembly_record: PathBuf,
    profile: String,
    sources: Vec<ServiceConfigSource>,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut artifact_root = None;
        let mut assembly_record = None;
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
                "--assembly-record" if assembly_record.is_none() => {
                    assembly_record = Some(PathBuf::from(value));
                }
                "--profile" if profile.is_none() => profile = Some(value),
                "--source" => {
                    let source = serde_json::from_str::<ServiceConfigSource>(&value)
                        .map_err(|error| format!("--source must be strict JSON: {error}"))?;
                    sources.push(source);
                }
                "--artifact-root" | "--assembly-record" | "--profile" => {
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
            assembly_record: assembly_record.ok_or("--assembly-record is required")?,
            profile: profile.ok_or("--profile is required")?,
            sources,
        })
    }
}

fn resolve_record(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("--assembly-record must be a normal relative artifact path".to_string());
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("inspect assembly record {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "assembly record {} must be a regular file",
            path.display()
        ));
    }
    Ok(path)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn discover_exact_packages(
    artifact_root: &Path,
    assembly: &RuntimeAssembly,
) -> Result<BTreeMap<PackageArtifactRef, PackageArtifact>, Box<dyn std::error::Error>> {
    let expected = assembly
        .resolved_packages
        .iter()
        .map(|reference| {
            (
                reference.package_build_id.as_str().to_string(),
                reference.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if expected.len() != assembly.resolved_packages.len() {
        return Err("assembly contains duplicate PackageBuildId references".into());
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
        let Some(expected_ref) = expected.get(build) else {
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
                "package record {} does not match assembly reference for {}",
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
    let missing = assembly
        .resolved_packages
        .iter()
        .filter(|reference| !found.contains_key(*reference))
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

#[allow(dead_code)]
fn _assert_receipt_is_serializable<T: Serialize>(_: &T) {}

#[cfg(test)]
mod tests;
