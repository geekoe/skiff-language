use super::*;

const MANIFEST_PACKAGE_ID: &str = "example.manifest";

fn source_site(source_id: u64) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: skiff_artifact_model::SourceSpanRef {
            source_id,
            start: skiff_artifact_model::SourcePosition::new(1, 1),
            end: skiff_artifact_model::SourcePosition::new(1, 2),
        },
    }
}

fn synthetic_site(reason: SyntheticInstructionSiteReason) -> InstructionSourceSite {
    InstructionSourceSite::Synthetic { reason }
}

fn statement_rich_bytecode() -> (
    Arc<ValidatedBytecodeArtifact>,
    PackageExecutableCoordinate,
    PackageCallableId,
) {
    let (bytecode, coordinate, callable) = callable_bytecode(false);
    let bytecode = mutate_function(&bytecode, "manifest::run", |function| {
        function.statement_entries = vec![
            StatementEntry {
                pc: 0,
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Generated { ordinal: 0 },
                site: synthetic_site(SyntheticInstructionSiteReason::RuntimeControlFlow),
            },
            StatementEntry {
                pc: 0,
                sequence_ordinal: 1,
                attribution_id: StatementAttributionId::Statement {
                    statement_index: 7,
                    occurrence_ordinal: 0,
                },
                site: source_site(11),
            },
            StatementEntry {
                pc: 1,
                sequence_ordinal: 0,
                attribution_id: StatementAttributionId::Expression {
                    expression_index: 9,
                    occurrence_ordinal: 0,
                },
                site: source_site(12),
            },
        ];
    });
    (bytecode, coordinate, callable)
}

fn mutate_function(
    bytecode: &ValidatedBytecodeArtifact,
    function_key: &str,
    mutate: impl FnOnce(&mut RelocatableBytecodeFunction),
) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = bytecode.artifact().clone();
    mutate(
        artifact
            .image
            .functions
            .get_mut(function_key)
            .expect("fixture function must exist"),
    );
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn package_for(
    bytecode: &Arc<ValidatedBytecodeArtifact>,
    coordinate: &PackageExecutableCoordinate,
    callable: &PackageCallableId,
) -> Arc<PackageArtifact> {
    callable_package(
        bytecode,
        coordinate,
        callable,
        OperationCallableKind::InternalFunction,
    )
}

fn package_with_declared_pin(
    bytecode: &Arc<ValidatedBytecodeArtifact>,
    coordinate: &PackageExecutableCoordinate,
    callable: &PackageCallableId,
    declared: BytecodeStatementManifestIdentity,
) -> Arc<PackageArtifact> {
    let mut artifact = package_for(bytecode, coordinate, callable).as_ref().clone();
    artifact.bytecode_statement_manifest_identity = declared;
    Arc::new(artifact)
}

fn assert_statement_mismatch(
    bytecode: Arc<ValidatedBytecodeArtifact>,
    coordinate: &PackageExecutableCoordinate,
    callable: &PackageCallableId,
    declared: BytecodeStatementManifestIdentity,
) {
    let artifact = package_with_declared_pin(&bytecode, coordinate, callable, declared);
    let error = HydratedBytecodePackage::checked(package_reference(&artifact), artifact, bytecode)
        .expect_err("statement manifest drift must fail closed");
    assert!(matches!(
        error,
        DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::StatementAttribution,
            ..
        }
    ));
}

fn assert_entry_drift(mutate: impl FnOnce(&mut Vec<StatementEntry>)) {
    let (baseline, coordinate, callable) = statement_rich_bytecode();
    let declared = statement_manifest_identity(MANIFEST_PACKAGE_ID, &baseline);
    let drifted = mutate_function(&baseline, "manifest::run", |function| {
        mutate(&mut function.statement_entries);
    });
    assert_statement_mismatch(drifted, &coordinate, &callable, declared);
}

#[test]
fn loader_accepts_exact_empty_and_nonempty_statement_manifests() {
    let empty_bytecode = admitted_bytecode("statement-empty");
    let empty_package = package_artifact(
        "example.empty",
        "build:empty",
        Some(empty_bytecode.reference().clone()),
    );
    assert_eq!(
        empty_package.bytecode_statement_manifest_identity,
        statement_manifest_identity("example.empty", &empty_bytecode)
    );
    HydratedBytecodePackage::checked(
        package_reference(&empty_package),
        empty_package,
        empty_bytecode,
    )
    .unwrap();

    let (bytecode, coordinate, callable) = statement_rich_bytecode();
    let package = package_for(&bytecode, &coordinate, &callable);
    assert_eq!(
        package.bytecode_statement_manifest_identity,
        statement_manifest_identity(MANIFEST_PACKAGE_ID, &bytecode)
    );
    HydratedBytecodePackage::checked(package_reference(&package), package, bytecode).unwrap();
}

#[test]
fn zero_event_function_is_included_in_the_statement_manifest() {
    let (bytecode, coordinate, callable) = callable_bytecode(false);
    let zero_event = mutate_function(&bytecode, "manifest::run", |function| {
        function.words = vec![0x25];
        function.statement_entries.clear();
    });
    let exact = package_for(&zero_event, &coordinate, &callable);
    let empty = derive_bytecode_statement_manifest_identity(MANIFEST_PACKAGE_ID, &[]).unwrap();
    assert_ne!(exact.bytecode_statement_manifest_identity, empty);
    HydratedBytecodePackage::checked(
        package_reference(&exact),
        Arc::clone(&exact),
        Arc::clone(&zero_event),
    )
    .unwrap();

    assert_statement_mismatch(zero_event, &coordinate, &callable, empty);
}

#[test]
fn statement_manifest_commits_pc_and_sequence_grouping() {
    assert_entry_drift(|entries| {
        entries[2].pc = 0;
        entries[2].sequence_ordinal = 2;
    });
}

#[test]
fn statement_manifest_commits_attribution_identity_and_site() {
    assert_entry_drift(|entries| {
        entries[1].attribution_id = StatementAttributionId::Statement {
            statement_index: 8,
            occurrence_ordinal: 0,
        };
    });
    assert_entry_drift(|entries| {
        entries[1].site = source_site(99);
    });
}

#[test]
fn statement_manifest_commits_same_pc_sequence_order() {
    assert_entry_drift(|entries| {
        entries.swap(0, 1);
        entries[0].sequence_ordinal = 0;
        entries[1].sequence_ordinal = 1;
    });
}

#[test]
fn statement_manifest_is_salted_by_exact_package_id() {
    let (bytecode, coordinate, callable) = statement_rich_bytecode();
    let wrong_salt = statement_manifest_identity("example.other", &bytecode);
    assert_statement_mismatch(bytecode, &coordinate, &callable, wrong_salt);
}

#[test]
fn statement_manifest_commits_synthetic_function_entries() {
    let (ordinary, coordinate, callable) = statement_rich_bytecode();
    let (baseline, callback_callable) =
        with_synthetic_callback(&ordinary, &coordinate, MANIFEST_PACKAGE_ID, &callable);
    let declared = statement_manifest_identity(MANIFEST_PACKAGE_ID, &baseline);
    let drifted = mutate_function(&baseline, "manifest::run$callback0", |function| {
        function.statement_entries[0].site =
            synthetic_site(SyntheticInstructionSiteReason::CompilerDesugaring);
    });
    let mut package = package_for(&drifted, &coordinate, &callable)
        .as_ref()
        .clone();
    add_synthetic_callback_owner(&mut package, &coordinate, &callback_callable);
    package.bytecode_statement_manifest_identity = declared;
    let package = Arc::new(package);

    let error = HydratedBytecodePackage::checked(package_reference(&package), package, drifted)
        .expect_err("synthetic statement drift must fail closed");
    assert!(matches!(
        error,
        DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::StatementAttribution,
            ..
        }
    ));
}

#[test]
fn loader_sorts_every_function_by_origin_instead_of_function_key() {
    let (ordinary, coordinate, callable) = statement_rich_bytecode();
    let (bytecode, callback_callable) =
        with_synthetic_callback(&ordinary, &coordinate, MANIFEST_PACKAGE_ID, &callable);
    let mut raw = bytecode.artifact().clone();
    let mut callback = raw
        .image
        .functions
        .remove("manifest::run$callback0")
        .unwrap();
    callback.function_key = "manifest::aaa".to_string();
    raw.image
        .functions
        .insert(callback.function_key.clone(), callback);
    skiff_artifact_identity::assign_bytecode_identity(&mut raw).unwrap();
    let bytecode = Arc::new(ValidatedBytecodeArtifact::admit(raw).unwrap());
    let viewed_origins = bytecode
        .view()
        .functions()
        .iter()
        .map(|function| function.origin.clone())
        .collect::<Vec<_>>();
    assert!(viewed_origins[0] > viewed_origins[1]);

    let mut package = package_for(&bytecode, &coordinate, &callable)
        .as_ref()
        .clone();
    add_synthetic_callback_owner(&mut package, &coordinate, &callback_callable);
    let package = Arc::new(package);
    HydratedBytecodePackage::checked(package_reference(&package), package, bytecode).unwrap();
}

#[test]
fn origin_and_callable_owner_errors_precede_statement_pin_mismatch() {
    let (bytecode, coordinate, callable) = statement_rich_bytecode();
    let wrong_pin = statement_manifest_identity("example.other", &bytecode);

    let mut missing_origin_owner =
        package_with_declared_pin(&bytecode, &coordinate, &callable, wrong_pin.clone())
            .as_ref()
            .clone();
    missing_origin_owner.files.clear();
    let missing_origin_owner = Arc::new(missing_origin_owner);
    assert!(matches!(
        HydratedBytecodePackage::checked(
            package_reference(&missing_origin_owner),
            missing_origin_owner,
            Arc::clone(&bytecode),
        ),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::FunctionOrigin,
            ..
        })
    ));

    let mut missing_canonical_owner =
        package_with_declared_pin(&bytecode, &coordinate, &callable, wrong_pin)
            .as_ref()
            .clone();
    missing_canonical_owner
        .package_local_abi
        .implementation_symbols
        .clear();
    let missing_canonical_owner = Arc::new(missing_canonical_owner);
    assert!(matches!(
        HydratedBytecodePackage::checked(
            package_reference(&missing_canonical_owner),
            missing_canonical_owner,
            bytecode,
        ),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::Callable,
            ..
        })
    ));
}

#[test]
fn synthetic_owner_error_precedes_statement_pin_mismatch() {
    let (ordinary, coordinate, callable) = statement_rich_bytecode();
    let (bytecode, _) =
        with_synthetic_callback(&ordinary, &coordinate, MANIFEST_PACKAGE_ID, &callable);
    let wrong_pin = statement_manifest_identity("example.other", &bytecode);
    let package = package_with_declared_pin(&bytecode, &coordinate, &callable, wrong_pin);

    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&package), package, bytecode),
        Err(DeploymentBytecodeHydrationError::ManifestMismatch {
            kind: DeploymentBytecodeManifestKind::SyntheticCallback,
            ..
        })
    ));
}

#[test]
fn bytecode_none_remains_missing_before_statement_validation() {
    let bytecode = admitted_bytecode("statement-none");
    let package = package_artifact("example.none", "build:none", None);
    assert_eq!(
        package.bytecode_statement_manifest_identity,
        derive_bytecode_statement_manifest_identity("example.none", &[]).unwrap()
    );
    assert!(matches!(
        HydratedBytecodePackage::checked(package_reference(&package), package, bytecode,),
        Err(DeploymentBytecodeHydrationError::MissingBytecode { .. })
    ));
}
