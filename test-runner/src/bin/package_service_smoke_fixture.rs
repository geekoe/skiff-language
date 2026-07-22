use std::{env, path::PathBuf, process};

use serde_json::json;
use skiff_artifact_identity::{
    package_artifact_ref, runtime_assembly_ref, PackageArtifactRecordPath,
};
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, RuntimeAssembly, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_compiler::CompilerPlatformSources;
use skiff_deployment::storage::{CanonicalArtifactStore, EnvironmentActivationState};
use skiff_test_runner::{
    canonical_fixture::discover_package_test_cases, canonical_package::compile_package_project,
    ecosystem_smoke_fixture::assemble_ecosystem_smoke_fixture,
    package_service_host_fixture::prepare_package_service_host_fixture,
    test_overlay::compile_package_test_overlay,
};

const USAGE: &str = "usage: skiff-package-service-smoke-fixture (<package-root> [--initialize-environment] | --bootstrap-only | --prepare-host-base <fixture-root> --work-root <dir> --receipt <file>) --artifact-root <dir> --environment <id> --platform-source-root <absolute-dir>";

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
    if let Some(fixture_root) = args.prepare_host_base.as_deref() {
        let receipt = prepare_package_service_host_fixture(
            &args.platform_sources,
            fixture_root,
            args.work_root
                .as_deref()
                .expect("prepare mode requires work root"),
            &args.artifact_root,
            &args.environment,
        )?;
        receipt.write(
            args.receipt
                .as_deref()
                .expect("prepare mode requires receipt path"),
        )?;
        println!("{}", serde_json::to_string(&receipt.to_json())?);
        return Ok(());
    }
    publish_candidate(args)
}

struct FixtureArgs {
    package_root: Option<PathBuf>,
    artifact_root: PathBuf,
    environment: String,
    platform_sources: CompilerPlatformSources,
    initialize_environment: bool,
    bootstrap_only: bool,
    prepare_host_base: Option<PathBuf>,
    work_root: Option<PathBuf>,
    receipt: Option<PathBuf>,
}

fn parse_args() -> anyhow::Result<FixtureArgs> {
    let mut package_root = None;
    let mut artifact_root = None;
    let mut environment = None;
    let mut platform_source_root = None;
    let mut initialize_environment = false;
    let mut bootstrap_only = false;
    let mut prepare_host_base = None;
    let mut work_root = None;
    let mut receipt = None;
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
            "--platform-source-root" => {
                set_once(
                    &mut platform_source_root,
                    PathBuf::from(next(&mut args, &argument)?),
                    &argument,
                )?;
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
            "--prepare-host-base" => {
                set_once(
                    &mut prepare_host_base,
                    PathBuf::from(next(&mut args, &argument)?),
                    &argument,
                )?;
            }
            "--work-root" => {
                set_once(
                    &mut work_root,
                    PathBuf::from(next(&mut args, &argument)?),
                    &argument,
                )?;
            }
            "--receipt" => {
                set_once(
                    &mut receipt,
                    PathBuf::from(next(&mut args, &argument)?),
                    &argument,
                )?;
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
    if prepare_host_base.is_some() {
        if bootstrap_only || package_root.is_some() || initialize_environment {
            anyhow::bail!(
                "--prepare-host-base is mutually exclusive with package, bootstrap, and initialization modes"
            );
        }
        if work_root.is_none() || receipt.is_none() {
            anyhow::bail!("--prepare-host-base requires --work-root and --receipt");
        }
    } else if work_root.is_some() || receipt.is_some() {
        anyhow::bail!("--work-root and --receipt require --prepare-host-base");
    }
    let platform_source_root =
        platform_source_root.ok_or_else(|| anyhow::anyhow!("missing --platform-source-root"))?;
    let platform_sources = CompilerPlatformSources::new(&platform_source_root)?;
    Ok(FixtureArgs {
        package_root,
        artifact_root: artifact_root.ok_or_else(|| anyhow::anyhow!("missing --artifact-root"))?,
        environment: environment.ok_or_else(|| anyhow::anyhow!("missing --environment"))?,
        platform_sources,
        initialize_environment,
        bootstrap_only,
        prepare_host_base,
        work_root,
        receipt,
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

    let project =
        compile_package_project(&args.platform_sources, &package_root, &args.artifact_root)?;
    let cases = discover_package_test_cases(&package_root, &package_root, false)?;
    if cases.is_empty() {
        anyhow::bail!("smoke fixture package must contain at least one .test.skiff case");
    }
    let overlay =
        compile_package_test_overlay(&args.platform_sources, &package_root, &project, &cases)?;
    let fixture = assemble_ecosystem_smoke_fixture(&project, overlay)?;
    fixture
        .records
        .publish(&args.artifact_root, &args.artifact_root)?;

    let store = CanonicalArtifactStore::open(&args.artifact_root)?;
    let bootstrap = if args.initialize_environment {
        Some(initialize_empty_environment(&store, &args.environment)?)
    } else {
        None
    };

    let assembly = runtime_assembly_ref(&fixture.records.assembly)?;
    let overlay_record_path = PackageArtifactRecordPath::new(&fixture.overlay)?.to_string();
    let production = package_artifact_ref(&project.package.artifact)?;
    let mut entrypoints = vec![
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
    ];
    if let Some(websocket) = fixture.websocket.as_ref() {
        entrypoints.push(json!({
            "kind": "websocket",
            "name": "websocket",
            "host": websocket.selector.host,
            "method": websocket.selector.method,
            "path": websocket.selector.path,
            "deployment": websocket.deployment,
            "contract": websocket.contract,
            "operation": websocket.operation,
        }));
    }
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
