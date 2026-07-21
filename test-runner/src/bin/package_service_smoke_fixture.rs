use std::{env, path::PathBuf, process};

use serde_json::json;
use skiff_artifact_identity::{
    package_artifact_ref, runtime_assembly_ref, PackageArtifactRecordPath,
};
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, RuntimeAssembly, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, EnvironmentActivationState};
use skiff_test_runner::{
    canonical_fixture::discover_package_test_cases,
    canonical_package::compile_package_project,
    ecosystem_smoke_fixture::{
        assemble_ecosystem_smoke_fixture, enable_ecosystem_smoke_server_stream,
    },
    test_overlay::compile_package_test_overlay,
};

const USAGE: &str = "usage: skiff-package-service-smoke-fixture (<package-root> [--initialize-environment] | --bootstrap-only) --artifact-root <dir> --environment <id>";

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        eprintln!("{USAGE}");
        process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args = parse_args()?;
    if args.bootstrap_only {
        return emit_bootstrap(&args.artifact_root, &args.environment);
    }
    publish_candidate(args)
}

struct FixtureArgs {
    package_root: Option<PathBuf>,
    artifact_root: PathBuf,
    environment: String,
    initialize_environment: bool,
    bootstrap_only: bool,
}

fn parse_args() -> anyhow::Result<FixtureArgs> {
    let mut package_root = None;
    let mut artifact_root = None;
    let mut environment = None;
    let mut initialize_environment = false;
    let mut bootstrap_only = false;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--artifact-root" => {
                set_once(
                    &mut artifact_root,
                    PathBuf::from(next(&mut args, &argument)?),
                    &argument,
                )?;
            }
            "--environment" => {
                set_once(&mut environment, next(&mut args, &argument)?, &argument)?;
            }
            "--initialize-environment" => {
                if initialize_environment {
                    anyhow::bail!("--initialize-environment was provided more than once");
                }
                initialize_environment = true;
            }
            "--bootstrap-only" => {
                if bootstrap_only {
                    anyhow::bail!("--bootstrap-only was provided more than once");
                }
                bootstrap_only = true;
            }
            value if value.starts_with('-') => anyhow::bail!("unknown option {value}"),
            value => set_once(&mut package_root, PathBuf::from(value), "package root")?,
        }
    }
    if bootstrap_only && (package_root.is_some() || initialize_environment) {
        anyhow::bail!(
            "--bootstrap-only does not accept a package root or --initialize-environment"
        );
    }
    Ok(FixtureArgs {
        package_root,
        artifact_root: artifact_root.ok_or_else(|| anyhow::anyhow!("missing --artifact-root"))?,
        environment: environment.ok_or_else(|| anyhow::anyhow!("missing --environment"))?,
        initialize_environment,
        bootstrap_only,
    })
}

fn emit_bootstrap(artifact_root: &std::path::Path, environment: &str) -> anyhow::Result<()> {
    let store = CanonicalArtifactStore::create(artifact_root)?;
    let bootstrap = initialize_empty_environment(&store, environment)?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schemaVersion": "skiff-package-service-bootstrap-v1",
            "environment": environment,
            "bootstrap": bootstrap,
        }))?
    );
    Ok(())
}

fn publish_candidate(args: FixtureArgs) -> anyhow::Result<()> {
    let package_root = args
        .package_root
        .ok_or_else(|| anyhow::anyhow!("missing package root"))?;

    let mut project = compile_package_project(&package_root, &[], &Default::default())?;
    enable_ecosystem_smoke_server_stream(&mut project)?;
    let cases = discover_package_test_cases(&package_root, &package_root, false)?;
    if cases.is_empty() {
        anyhow::bail!("smoke fixture package must contain at least one .test.skiff case");
    }
    let overlay = compile_package_test_overlay(&package_root, &project, &cases, &[])?;
    let fixture = assemble_ecosystem_smoke_fixture(&project, overlay)?;
    fixture.records.publish(&args.artifact_root)?;

    let store = CanonicalArtifactStore::open(&args.artifact_root)?;
    let bootstrap = if args.initialize_environment {
        Some(initialize_empty_environment(&store, &args.environment)?)
    } else {
        None
    };

    let assembly = runtime_assembly_ref(&fixture.records.assembly)?;
    let overlay_record_path = PackageArtifactRecordPath::new(&fixture.overlay)?.to_string();
    let production = package_artifact_ref(&project.package.artifact)?;
    let entrypoints = [
        json!({
            "kind": "packageTest",
            "name": fixture.package_test.case.name,
            "host": fixture.package_test.selector.host,
            "method": fixture.package_test.selector.method,
            "path": fixture.package_test.selector.path,
            "deployment": fixture.package_test.deployment,
            "contract": fixture.package_test.contract,
            "operation": fixture.package_test.operation,
        }),
        json!({
            "kind": "unary",
            "name": "marker",
            "host": fixture.unary.selector.host,
            "method": fixture.unary.selector.method,
            "path": fixture.unary.selector.path,
            "deployment": fixture.unary.deployment,
            "contract": fixture.unary.contract,
            "operation": fixture.unary.operation,
        }),
        json!({
            "kind": "serverStream",
            "name": "events",
            "host": fixture.stream.selector.host,
            "method": fixture.stream.selector.method,
            "path": fixture.stream.selector.path,
            "deployment": fixture.stream.deployment,
            "contract": fixture.stream.contract,
            "operation": fixture.stream.operation,
        }),
    ];
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schemaVersion": "skiff-package-service-smoke-fixture-v1",
            "environment": args.environment,
            "bootstrap": bootstrap,
            "candidate": {
                "assembly": assembly,
                "production": production,
                "overlay": fixture.overlay,
                "overlayRecordPath": overlay_record_path,
                "entrypoints": entrypoints,
            },
        }))?
    );
    Ok(())
}

fn initialize_empty_environment(
    store: &CanonicalArtifactStore,
    environment: &str,
) -> anyhow::Result<serde_json::Value> {
    let mut empty = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut empty)?;
    store.write_runtime_assembly(&empty)?;
    let reference = runtime_assembly_ref(&empty)?;
    store.initialize_environment_activation(&EnvironmentActivationState::initial(
        environment,
        0,
        reference.clone(),
    ))?;
    Ok(json!({ "generation": 0, "assembly": reference }))
}

fn next(args: &mut impl Iterator<Item = String>, option: &str) -> anyhow::Result<String> {
    args.next()
        .ok_or_else(|| anyhow::anyhow!("{option} requires a value"))
}

fn set_once<T>(target: &mut Option<T>, value: T, label: &str) -> anyhow::Result<()> {
    if target.replace(value).is_some() {
        anyhow::bail!("{label} was provided more than once");
    }
    Ok(())
}
