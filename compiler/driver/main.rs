use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use skiff_compiler::{
    build_service_publication, classify_publication_root, collect_source_tree,
    read_service_config_with_profile, PackageResolutionDirs, PublicationId, PublicationInputError,
    PublicationInputKind, PublishedArtifactVisitOptions, PublishedServiceArtifacts, ServiceConfig,
    ServicePublicationBuildInput, SkiffRuntimeManifest, SourceTree, SERVICE_CONFIG_FILE,
};

const USAGE: &str = "usage: skiff-compiler [--test] [--live --allow-network --config <config-json>] <service-root> [--out <artifact-json>] [--manifest-out <router-manifest-json>] [--assembly-out <service-assembly-json>] [--ir-out <deprecated-service-assembly-alias>] [--artifact-root <dir>] [--service-id <id>] [--profile <name>] [--packages-dir <dir>]... [--service-artifact-root <dir>]...";

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
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<(), CliError> {
    let args = parse_args(args)?;
    if args.test {
        if args.out.is_some()
            || args.manifest_out.is_some()
            || args.ir_out.is_some()
            || args.assembly_out.is_some()
            || args.artifact_root.is_some()
            || args.service_id.is_some()
            || !args.service_dependency_artifact_roots.is_empty()
        {
            return Err(CliError::message(
                "--test cannot be combined with artifact output options",
            ));
        }
        let _ = (args.live, args.allow_network, args.config_path.as_ref());
        return Err(CliError::message(
            "--test has moved out of skiff-compiler; use skiff-test-runner instead",
        ));
    }
    let service_input = resolve_input(&args.input, args.profile.as_deref())?;
    let package_dirs = package_resolution_dirs(args.package_dirs.clone());
    let published = build_service_publication(ServicePublicationBuildInput {
        config: &service_input.config,
        source_tree: &service_input.source_tree,
        service_id_override: args.service_id.as_deref(),
        package_dirs,
        service_dependency_artifact_roots: &args.service_dependency_artifact_roots,
    })
    .map_err(|error| CliError::message(format!("service publication build failed: {error}")))?;
    write_json_artifact(
        args.out.as_ref().expect("--out is required outside --test"),
        &published.artifacts.service_assembly.value,
        "service assembly",
    )?;
    if let Some(manifest_out) = &args.manifest_out {
        write_manifest(manifest_out, &published.manifest)?;
    }
    if let Some(ir_out) = &args.ir_out {
        write_service_assembly(ir_out, &published.artifacts.service_assembly.value)?;
    }
    if let Some(assembly_out) = &args.assembly_out {
        write_service_assembly(assembly_out, &published.artifacts.service_assembly.value)?;
    }
    if let Some(artifact_root) = &args.artifact_root {
        write_artifact_root(artifact_root, &published.artifacts)?;
    }
    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, CliError> {
    let mut input = None;
    let mut out = None;
    let mut manifest_out = None;
    let mut ir_out = None;
    let mut assembly_out = None;
    let mut artifact_root = None;
    let mut service_id = None;
    let mut package_dirs = Vec::new();
    let mut service_dependency_artifact_roots = Vec::new();
    let mut profile = None;
    let mut test = false;
    let mut live = false;
    let mut allow_network = false;
    let mut config_path = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(CliError::Help),
            "--test" => {
                if test {
                    return Err(CliError::message("--test was provided more than once"));
                }
                test = true;
            }
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
            "-o" | "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--out requires a path"))?;
                if out.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message("--out was provided more than once"));
                }
            }
            "--manifest-out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--manifest-out requires a path"))?;
                if manifest_out.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message(
                        "--manifest-out was provided more than once",
                    ));
                }
            }
            "--ir-out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--ir-out requires a path"))?;
                if ir_out.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message("--ir-out was provided more than once"));
                }
            }
            "--assembly-out" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--assembly-out requires a path"))?;
                if assembly_out.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message(
                        "--assembly-out was provided more than once",
                    ));
                }
            }
            "--artifact-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--artifact-root requires a path"))?;
                if artifact_root.replace(PathBuf::from(value)).is_some() {
                    return Err(CliError::message(
                        "--artifact-root was provided more than once",
                    ));
                }
            }
            "--service-id" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::message("--service-id requires an id"))?;
                if value.is_empty() {
                    return Err(CliError::message("--service-id cannot be empty"));
                }
                let service_publication_id = PublicationId::parse(&value).map_err(|error| {
                    CliError::message(format!("--service-id is invalid: {error}"))
                })?;
                if service_id
                    .replace(service_publication_id.into_string())
                    .is_some()
                {
                    return Err(CliError::message(
                        "--service-id was provided more than once",
                    ));
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
                service_dependency_artifact_roots.push(PathBuf::from(value));
            }
            _ if arg.starts_with("--service-artifact-root=") => {
                service_dependency_artifact_roots
                    .push(PathBuf::from(&arg["--service-artifact-root=".len()..]));
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
    if !test && out.is_none() {
        return Err(CliError::message("missing --out path"));
    }
    if !test && (live || allow_network || config_path.is_some()) {
        return Err(CliError::message(
            "--live, --allow-network, and --config require --test",
        ));
    }
    Ok(CliArgs {
        input,
        out,
        manifest_out,
        ir_out,
        assembly_out,
        artifact_root,
        service_id,
        package_dirs,
        service_dependency_artifact_roots,
        profile,
        test,
        live,
        allow_network,
        config_path,
    })
}

fn resolve_input(input: &Path, profile: Option<&str>) -> Result<ServicePublicationInput, CliError> {
    let metadata = fs::metadata(input).map_err(|error| {
        CliError::message(format!(
            "failed to inspect input {}: {error}",
            input.display()
        ))
    })?;

    if metadata.is_file() {
        return Err(CliError::message(format!(
            "input {} is a file; skiff-compiler expects a service publication root containing {SERVICE_CONFIG_FILE}. Use the test runner or package tooling for package/test file selection.",
            input.display()
        )));
    }

    if metadata.is_dir() {
        let root_manifest = match classify_publication_root(input) {
            Ok(root_manifest) => root_manifest,
            Err(PublicationInputError::MissingRootManifest { .. }) => {
                return Err(CliError::message(format!(
                    "directory input {} is not a service publication root; expected {SERVICE_CONFIG_FILE}. Use the test runner or package tooling for package roots.",
                    input.display()
                )));
            }
            Err(error) => return Err(CliError::message(error.to_string())),
        };

        match root_manifest.kind() {
            PublicationInputKind::Service => {
                let config = read_service_config_with_profile(input, profile)
                    .map_err(|error| CliError::message(format!("service config error: {error}")))?;
                let source_tree = collect_source_tree(input)
                    .map_err(|error| CliError::message(format!("source tree error: {error}")))?;
                return Ok(ServicePublicationInput {
                    config,
                    source_tree,
                });
            }
            PublicationInputKind::Package => {
                return Err(CliError::message(format!(
                    "directory input {} is a package publication root; skiff-compiler expects a service publication root containing {SERVICE_CONFIG_FILE}. Use package tooling for package roots.",
                    input.display()
                )));
            }
        }
    }

    Err(CliError::message(format!(
        "input {} is neither a file nor a directory",
        input.display()
    )))
}

fn package_resolution_dirs(package_dirs: Vec<PathBuf>) -> PackageResolutionDirs {
    PackageResolutionDirs { package_dirs }
}

fn write_manifest(path: &Path, manifest: &SkiffRuntimeManifest) -> Result<(), CliError> {
    write_pretty_json(path, manifest, "manifest")
}

fn write_service_assembly(
    path: &Path,
    service_assembly: &serde_json::Value,
) -> Result<(), CliError> {
    write_json_artifact(path, service_assembly, "service assembly")
}

fn write_json_artifact(
    path: &Path,
    artifact: &serde_json::Value,
    kind: &str,
) -> Result<(), CliError> {
    write_pretty_json(path, artifact, kind)
}

fn write_artifact_root(root: &Path, artifacts: &PublishedServiceArtifacts) -> Result<(), CliError> {
    artifacts.try_visit_json_artifacts(
        PublishedArtifactVisitOptions {
            include_contract_schema: true,
        },
        |artifact| write_json_artifact(&root.join(&artifact.path), &artifact.value, artifact.kind),
    )
}

fn write_pretty_json<T>(path: &Path, value: &T, kind: &str) -> Result<(), CliError>
where
    T: serde::Serialize,
{
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            CliError::message(format!(
                "failed to create output directory {}: {error}",
                parent.display()
            ))
        })?;
    }

    let json = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::message(format!("failed to serialize {kind}: {error}")))?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| CliError::message(format!("failed to write {}: {error}", path.display())))
}

struct CliArgs {
    input: PathBuf,
    out: Option<PathBuf>,
    manifest_out: Option<PathBuf>,
    ir_out: Option<PathBuf>,
    assembly_out: Option<PathBuf>,
    artifact_root: Option<PathBuf>,
    service_id: Option<String>,
    package_dirs: Vec<PathBuf>,
    service_dependency_artifact_roots: Vec<PathBuf>,
    profile: Option<String>,
    test: bool,
    live: bool,
    allow_network: bool,
    config_path: Option<PathBuf>,
}

struct ServicePublicationInput {
    config: ServiceConfig,
    source_tree: SourceTree,
}

enum CliError {
    Help,
    Message(String),
}

impl CliError {
    fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
