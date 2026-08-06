use std::{
    env,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use skiff_artifact_identity::{
    runtime_assembly_ref, service_deployment_ref, PackageArtifactRecordPath,
};
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, GatewayDispatchMode, RuntimeAssembly,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_compiler::CompilerPlatformSources;
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_runtime_config_snapshot::{
    new_runtime_config_snapshot_ref, RuntimeConfigSnapshot, RuntimeConfigSnapshotStore,
};
use skiff_test_runner::{
    canonical_fixture::discover_test_service_cases,
    canonical_package::compile_package_project_for_test,
    canonical_std_seed::{seed_canonical_std, CanonicalStdSeedReceipt},
    package_service_host_fixture::prepare_package_service_host_fixture,
    test_service_fixture::{
        assemble_test_service_fixture, assemble_test_service_fixture_for_run_with_ingress,
        CanonicalTestServiceFixture,
    },
};

const USAGE: &str = "usage: skiff-package-service-smoke-fixture (<package-root> [--initialize-profile | --seed-committed] | --bootstrap-only | --prepare-host-base <fixture-root> --work-root <dir> --receipt <file>) --artifact-root <dir> --profile <id> --platform-source-root <absolute-dir>";

/// Canonical artifact record path of the A3 actor routing projection
/// (`router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH`); the strict
/// loader fails closed when this record is missing from the artifact root.
const ACTOR_ROUTING_PROJECTION_RECORD_PATH: &str = "records/actor-routing/current.json";

/// Canonical JSON bytes of the empty A0 actor routing projection
/// (`ActorRoutingProjection::new("skiff-actor-routing-projection-v1", [])`).
/// Keys are sorted and the JSON is compact, matching the strict loader's
/// canonical-bytes equality check.
const EMPTY_ACTOR_ROUTING_PROJECTION_RECORD: &str =
    r#"{"methods":[],"schemaVersion":"skiff-actor-routing-projection-v1"}"#;

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
        return emit_bootstrap(&args.platform_sources, &args.artifact_root, &args.profile);
    }
    if let Some(fixture_root) = args.prepare_host_base.as_deref() {
        let receipt = prepare_package_service_host_fixture(
            &args.platform_sources,
            fixture_root,
            args.work_root
                .as_deref()
                .expect("prepare mode requires work root"),
            &args.artifact_root,
            &args.profile,
        )?;
        receipt.write(
            args.receipt
                .as_deref()
                .expect("prepare mode requires receipt path"),
        )?;
        println!("{}", serde_json::to_string(&receipt.to_json())?);
        return Ok(());
    }
    if args.seed_committed {
        let package_root = args
            .package_root
            .as_deref()
            .expect("--seed-committed was validated to carry a package root");
        return seed_committed(
            &args.platform_sources,
            package_root,
            &args.artifact_root,
            &args.profile,
        );
    }
    publish_candidate(args)
}

struct FixtureArgs {
    package_root: Option<PathBuf>,
    artifact_root: PathBuf,
    profile: String,
    platform_sources: CompilerPlatformSources,
    initialize_profile: bool,
    bootstrap_only: bool,
    seed_committed: bool,
    prepare_host_base: Option<PathBuf>,
    work_root: Option<PathBuf>,
    receipt: Option<PathBuf>,
}

fn parse_args() -> anyhow::Result<FixtureArgs> {
    let mut package_root = None;
    let mut artifact_root = None;
    let mut profile = None;
    let mut platform_source_root = None;
    let mut initialize_profile = false;
    let mut bootstrap_only = false;
    let mut seed_committed = false;
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
            "--profile" => {
                set_once(&mut profile, next(&mut args, &argument)?, &argument)?;
            }
            "--platform-source-root" => {
                set_once(
                    &mut platform_source_root,
                    PathBuf::from(next(&mut args, &argument)?),
                    &argument,
                )?;
            }
            "--initialize-profile" => {
                if initialize_profile {
                    anyhow::bail!("--initialize-profile was provided more than once");
                }
                initialize_profile = true;
            }
            "--bootstrap-only" => {
                if bootstrap_only {
                    anyhow::bail!("--bootstrap-only was provided more than once");
                }
                bootstrap_only = true;
            }
            "--seed-committed" => {
                if seed_committed {
                    anyhow::bail!("--seed-committed was provided more than once");
                }
                seed_committed = true;
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
    if bootstrap_only && (package_root.is_some() || initialize_profile) {
        anyhow::bail!("--bootstrap-only does not accept a package root or --initialize-profile");
    }
    if bootstrap_only && seed_committed {
        anyhow::bail!("--bootstrap-only and --seed-committed are mutually exclusive");
    }
    if prepare_host_base.is_some() {
        if bootstrap_only || seed_committed || package_root.is_some() || initialize_profile {
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
    if seed_committed && (package_root.is_none() || initialize_profile) {
        anyhow::bail!("--seed-committed requires a package root and rejects --initialize-profile");
    }
    let platform_source_root =
        platform_source_root.ok_or_else(|| anyhow::anyhow!("missing --platform-source-root"))?;
    let platform_sources = CompilerPlatformSources::new(&platform_source_root)?;
    Ok(FixtureArgs {
        package_root,
        artifact_root: artifact_root.ok_or_else(|| anyhow::anyhow!("missing --artifact-root"))?,
        profile: profile.ok_or_else(|| anyhow::anyhow!("missing --profile"))?,
        platform_sources,
        initialize_profile,
        bootstrap_only,
        seed_committed,
        prepare_host_base,
        work_root,
        receipt,
    })
}

fn emit_bootstrap(
    platform_sources: &CompilerPlatformSources,
    artifact_root: &std::path::Path,
    profile: &str,
) -> anyhow::Result<()> {
    let std = seed_canonical_std(platform_sources, artifact_root)?;
    let store = CanonicalArtifactStore::create(artifact_root)?;
    let bootstrap = initialize_empty_profile(&store, profile)?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schemaVersion": "skiff-package-service-bootstrap-v3",
            "profile": profile,
            "bootstrap": {
                "assembly": bootstrap["assembly"],
                "configSnapshot": bootstrap["configSnapshot"],
                "std": std.to_json(),
            },
        }))?
    );
    Ok(())
}

fn publish_candidate(args: FixtureArgs) -> anyhow::Result<()> {
    let package_root = args
        .package_root
        .ok_or_else(|| anyhow::anyhow!("missing package root"))?;
    let (_std, fixture) = assemble_fixture_candidate(
        &args.platform_sources,
        &package_root,
        &args.artifact_root,
        &args.profile,
    )?;
    let case = fixture
        .cases
        .first()
        .ok_or_else(|| anyhow::anyhow!("smoke fixture requires at least one test case"))?;
    fixture.publish(&args.artifact_root, &args.artifact_root)?;

    let store = CanonicalArtifactStore::open(&args.artifact_root)?;
    let bootstrap = if args.initialize_profile {
        Some(initialize_empty_profile(&store, &args.profile)?)
    } else {
        None
    };

    let assembly = runtime_assembly_ref(&fixture.records.assembly)?;
    let test_service_record_path =
        PackageArtifactRecordPath::new(&fixture.test_service)?.to_string();
    let test_entrypoint = &case.entrypoint;
    let deployment = fixture
        .records
        .deployments
        .first()
        .ok_or_else(|| anyhow::anyhow!("smoke fixture case omitted its ordinary deployment"))?;
    let _probe_ingress = deployment
        .ingress
        .iter()
        .find(|binding| binding.selector.path == "/probe")
        .ok_or_else(|| anyhow::anyhow!("smoke fixture service must declare /probe in http.yml"))?;
    let _probe_entry = deployment
        .gateway_entries
        .get(&_probe_ingress.gateway_entry_key)
        .ok_or_else(|| anyhow::anyhow!("smoke fixture /probe ingress has no gateway entry"))?;
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(deployment);
    let mut entrypoints = vec![json!({
        "deployment": test_entrypoint.deployment,
        "gatewayEntryKey": test_entrypoint.gateway_entry_key,
        "gatewayEntryIdentity": test_entrypoint.gateway_entry_identity,
        "mode": test_entrypoint.mode,
        "selector": test_entrypoint.selector,
    })];
    for (gateway_entry_key, gateway_entry) in &deployment.gateway_entries {
        if gateway_entry_key == &test_entrypoint.gateway_entry_key {
            continue;
        }
        let selector = deployment
            .ingress
            .iter()
            .find(|binding| binding.gateway_entry_key == *gateway_entry_key)
            .map(|binding| binding.selector.clone());
        entrypoints.push(json!({
            "deployment": deployment_ref,
            "gatewayEntryKey": gateway_entry_key,
            "gatewayEntryIdentity": gateway_entry.gateway_entry_identity,
            "mode": GatewayDispatchMode::Unary,
            "selector": selector,
        }));
    }
    let contracts = fixture
        .records
        .contracts
        .iter()
        .map(skiff_artifact_identity::service_contract_ref)
        .collect::<Result<Vec<_>, _>>()?;
    let deployments = fixture
        .records
        .deployments
        .iter()
        .map(skiff_artifact_identity::service_deployment_ref)
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schemaVersion": "skiff-package-service-smoke-fixture-v4",
            "profile": args.profile,
            "bootstrap": bootstrap,
            "candidate": {
                "assembly": assembly,
                "configSnapshot": fixture.records.config_snapshot.snapshot_ref(),
                "testService": fixture.test_service,
                "testServiceRecordPath": test_service_record_path,
                "contracts": contracts,
                "deployments": deployments,
                "entrypoints": entrypoints,
            },
        }))?
    );
    Ok(())
}

/// Assembles one canonical test-service fixture candidate for the smoke
/// fixture modes: seeds std, compiles the package for test, discovers its
/// `.test.skiff` cases and builds the immutable fixture records (package,
/// contracts, deployments, assembly, config snapshot).
fn assemble_fixture_candidate(
    platform_sources: &CompilerPlatformSources,
    package_root: &Path,
    artifact_root: &Path,
    profile: &str,
) -> anyhow::Result<(CanonicalStdSeedReceipt, CanonicalTestServiceFixture)> {
    let std = seed_canonical_std(platform_sources, artifact_root)?;
    let project = compile_package_project_for_test(platform_sources, package_root, artifact_root)?;
    let cases = discover_test_service_cases(package_root, package_root, false)?;
    if cases.is_empty() {
        anyhow::bail!("smoke fixture package must contain at least one .test.skiff case");
    }
    let run_scope = format!(
        "package-service-smoke-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock after Unix epoch")
            .as_nanos()
    );
    let fixture = match env::var("SKIFF_TEST_INGRESS_URL").ok() {
        Some(ingress_url) => assemble_test_service_fixture_for_run_with_ingress(
            &project,
            &cases,
            Default::default(),
            &run_scope,
            &ingress_url,
            profile,
        )?,
        None => assemble_test_service_fixture(&project, &cases, Default::default(), profile)?,
    };
    Ok((std, fixture))
}

/// Seeds one fixture package as the current release set for its profile:
/// publishes the fixture's immutable records (service deployments, assembly,
/// config snapshot), writes the canonical actor routing projection record and
/// sets the release pointer for every published deployment. The emitted
/// `skiff-package-service-bootstrap-v3` receipt drives the isolated release
/// seed, so the router resolves the profile's releases without any
/// coordination state.
fn seed_committed(
    platform_sources: &CompilerPlatformSources,
    package_root: &Path,
    artifact_root: &Path,
    profile: &str,
) -> anyhow::Result<()> {
    let (std, fixture) =
        assemble_fixture_candidate(platform_sources, package_root, artifact_root, profile)?;
    fixture.publish(artifact_root, artifact_root)?;
    write_actor_routing_projection(artifact_root)?;
    let store = CanonicalArtifactStore::open(artifact_root)?;
    for deployment in &fixture.records.deployments {
        let deployment_ref = service_deployment_ref(deployment);
        let pointer = ReleasePointer::new(profile, deployment_ref.clone())?;
        store.write_release_pointer(&pointer)?;
    }
    let assembly_ref = runtime_assembly_ref(&fixture.records.assembly)?;
    let config_snapshot_ref = fixture.records.config_snapshot.snapshot_ref().clone();
    println!(
        "{}",
        serde_json::to_string(&json!({
            "schemaVersion": "skiff-package-service-bootstrap-v3",
            "profile": profile,
            "bootstrap": {
                "assembly": assembly_ref,
                "configSnapshot": config_snapshot_ref,
                "std": std.to_json(),
            },
        }))?
    );
    Ok(())
}

/// Writes the canonical empty A0 actor routing projection record required by
/// the E-bootstrap strict loader (`records/actor-routing/current.json`).
fn write_actor_routing_projection(artifact_root: &Path) -> anyhow::Result<()> {
    let actor_routing_record = artifact_root.join(ACTOR_ROUTING_PROJECTION_RECORD_PATH);
    std::fs::create_dir_all(
        actor_routing_record
            .parent()
            .expect("actor routing record path has a parent directory"),
    )?;
    std::fs::write(&actor_routing_record, EMPTY_ACTOR_ROUTING_PROJECTION_RECORD)?;
    Ok(())
}

fn initialize_empty_profile(
    store: &CanonicalArtifactStore,
    profile: &str,
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
        gateway_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut empty)?;
    store.write_runtime_assembly(&empty)?;
    // The E-bootstrap strict loader requires the canonical actor routing
    // projection record at the artifact root; the empty profile carries an
    // empty projection until the A1 producer publishes real routing facts.
    write_actor_routing_projection(store.root())?;
    let reference = runtime_assembly_ref(&empty)?;
    let config_snapshot_ref = new_runtime_config_snapshot_ref();
    let config_snapshot =
        RuntimeConfigSnapshot::new(profile, config_snapshot_ref.clone(), Vec::new())?;
    RuntimeConfigSnapshotStore::create(store.root().join("runtime-config"))?
        .publish(&config_snapshot)?;
    // The empty profile is a baseline with an empty release pointer table;
    // no coordination state is written for it.
    Ok(json!({
        "assembly": reference,
        "configSnapshot": config_snapshot_ref,
    }))
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
