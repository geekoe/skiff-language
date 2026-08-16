use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_bytecode_identity, assign_package_artifact_identities, package_schema_index_identity,
    BYTECODE_IDENTITY_PREFIX, BYTECODE_IDENTITY_SCHEMA_MARKER, FILE_IR_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX, PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
};
use skiff_artifact_model::{
    current_platform_error_projection_registry_ref, descriptor_for_opcode,
    validate_current_platform_error_projection_registry_ref,
    validate_platform_error_projection_registry_ref_shape, BytecodeArtifact, BytecodeArtifactRef,
    BytecodeFunctionOrigin, BytecodeFunctionStatementManifest, BytecodeImage, BytecodePoolEntry,
    BytecodePools, FrameLayout, FrozenConstantGraph, InstructionSourceSite, Opcode, OperandRole,
    PackageBuildId, PackageCallableId, PackageExecutableCoordinate, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndex,
    PackageSchemaIndexRef, PlatformErrorProjectionRegistryRef, RelocatableBytecodeFunction,
    SourcePosition, SourceSpanRef, StatementAttributionId, StatementEntry, WritablePathSegment,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_compiler_emission::package_artifact::publish_projected_package_artifact;

use super::*;

const PACKAGE_ID: &str = "example.com/bytecode-attachment";
const PRODUCTION_SERVER_STREAM_SOURCE: &str = r#"import std

function consume(
  request: std.http.HttpRequest
) -> Stream<std.http.HttpResponseStreamEvent> {
  final outbound = std.http.HttpClientRequest {
    method: request.method,
    url: request.body.toUtf8String(),
    headers: request.headers,
    body: null,
    timeoutMs: null,
  }
  final response = std.http.stream(outbound)
  emit({
    tag: "start",
    status: 207,
    headers: [],
  })
  for chunk in response.body {
    emit({ tag: "chunk", value: chunk })
  }
  emit({ tag: "end" })
  return null
}
"#;

const PRODUCTION_DIVERGING_CATCH_SOURCE: &str = r#"
type LeafA {
  marker: number,
  owner: Array<number>,
}

type LeafB {
  marker: number,
  owner: Array<number>,
}

function innerThrow(leaf: LeafA) -> void {
  final cleanupOwner = [7]
  throw leaf
}

function direct(seed: number) -> number {
  final attempted = catch<LeafB>(throw LeafA { marker: seed, owner: [seed] })
  if attempted.tag == "ok" {
    return 7
  }
  return 99
}

function rethrowing(seed: number) -> number {
  if seed == 1 {
    final leaf = LeafA { marker: seed, owner: [seed] }
    final inner = catch<LeafA>(innerThrow(leaf))
    if inner.tag == "err" {
      final exception = inner.exception
      final outer = catch<LeafA>(rethrow exception)
      if outer.tag == "err" {
        return 2
      }
      return 11
    }
  }
  return 12
}
"#;

const PRODUCTION_NESTED_WRITABLE_SOURCE: &str = r#"
type Cell { count: integer }
type State { rows: Array<Cell> }

function rewrite() -> number {
  var state = State { rows: [Cell { count: 0 }] }
  state.rows[0].count = 7
  return state.rows[0].count
}
"#;

#[test]
fn explicitly_disabled_outcome_is_the_only_none_lane() {
    let lane = finish_bytecode_lane(BytecodeCompilationOutcome::disabled()).unwrap();

    assert_eq!(lane, PackageBytecodeLane::Disabled);
    assert!(!lane.is_enabled());
    assert!(lane.handoff().is_none());
    assert!(lane.receipt().is_none());
}

#[test]
fn enabled_failure_is_propagated_instead_of_becoming_disabled() {
    let error = finish_bytecode_lane(BytecodeCompilationOutcome::failed(
        PackageCompileError::BytecodeEmitterUnavailable { mir_unit_count: 3 },
    ))
    .unwrap_err();

    assert!(matches!(
        error,
        PackageCompileError::BytecodeEmitterUnavailable { mir_unit_count: 3 }
    ));
}

#[test]
fn disabled_lane_requires_the_package_specific_empty_manifest() {
    let projected = projected_fixture(PACKAGE_ID);
    let source = projected.artifact.clone();

    let result = attach_bytecode_execution(&projected, &PackageBytecodeLane::Disabled).unwrap();

    assert_eq!(projected.artifact, source);
    assert_eq!(result.artifact, source);
    assert!(result.artifact.bytecode.is_none());
    assert_eq!(
        result.artifact.bytecode_statement_manifest_identity,
        derive_bytecode_statement_manifest_identity(PACKAGE_ID, &[]).unwrap()
    );
}

#[test]
fn enabled_lane_attaches_exact_handoff_ref_and_manifest_to_a_new_projection() {
    let projected = projected_fixture(PACKAGE_ID);
    let source = projected.artifact.clone();
    let lane = enabled_lane(PACKAGE_ID);
    let handoff = lane.handoff().unwrap();

    let attached = attach_bytecode_execution(&projected, &lane).unwrap();

    assert_eq!(projected.artifact, source);
    assert_eq!(attached.artifact.package_id, PACKAGE_ID);
    assert_eq!(
        attached.artifact.bytecode.as_ref(),
        Some(handoff.reference())
    );
    assert_eq!(
        &attached.artifact.bytecode_statement_manifest_identity,
        handoff.statement_manifest_receipt().identity()
    );
    assert_eq!(
        attached.artifact.platform_error_projection_registry,
        source.platform_error_projection_registry
    );
    assert_eq!(
        &attached.artifact.platform_error_projection_registry,
        handoff
            .receipt()
            .authorities()
            .platform_error_projection_registry()
    );
    validate_current_platform_error_projection_registry_ref(
        &attached.artifact.platform_error_projection_registry,
    )
    .unwrap();
    assert_eq!(handoff.statement_manifest_receipt().function_count(), 2);
    assert_eq!(handoff.statement_manifest_receipt().event_count(), 2);
    assert_eq!(
        attached.artifact.package_local_abi.local_abi_identity,
        source.package_local_abi.local_abi_identity
    );
    assert_ne!(attached.artifact.package_build_id, source.package_build_id);
    assert!(handoff
        .reference()
        .bytecode_identity
        .starts_with("skiff-bytecode-image-v5:sha256:"));
    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v14");
    assert_eq!(
        BYTECODE_IDENTITY_SCHEMA_MARKER,
        "skiff-bytecode-artifact-v5"
    );
    assert_eq!(BYTECODE_IDENTITY_PREFIX, "skiff-bytecode-image-v5:sha256");
    assert_eq!(
        PACKAGE_ARTIFACT_SCHEMA_VERSION,
        "skiff-package-artifact-v15"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
        "skiff-package-artifact-build-identity-v13"
    );
    assert_eq!(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        "skiff-package-build-v14:sha256"
    );
}

#[test]
fn production_authoring_publishes_exact_affine_http_stream_bytecode() {
    let repository_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler manifest has repository root")
        .to_path_buf();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock follows Unix epoch")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!(
        "skiff-c5-production-affine-{}-{nonce}",
        std::process::id()
    ));
    let package_root = temp.join("package");
    let artifact_root = temp.join("artifacts");
    std::fs::create_dir_all(&package_root).expect("create production fixture root");
    for (path, contents) in [
        (
            "package.yml",
            "id: test.skiff/c5-production-affine\nversion: 1.0.0\n",
        ),
        ("service.yml", "id: test.skiff/c5-production-affine\n"),
        ("api.yml", "{}\n"),
        (
            "http.yml",
            "consume:\n  method: POST\n  path: /phase-5/compiler\n  kind: rawHttp\n  handler: main.consume\n  adapterArgs:\n    - param: request\n      source: { kind: http.request }\n",
        ),
        (
            "main.skiff",
            PRODUCTION_SERVER_STREAM_SOURCE,
        ),
    ] {
        std::fs::write(package_root.join(path), contents).expect("write production fixture");
    }

    let platform = crate::CompilerPlatformSources::new(&repository_root)
        .expect("open production platform sources");
    crate::authoring::seed_official_std_package(&platform, &artifact_root)
        .expect("seed exact compiler-owned std package");
    let receipt = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect("production authoring must consume resolved std package authority");
    let package_ref: skiff_artifact_model::PackageArtifactRef =
        serde_json::from_value(receipt["packageArtifactReceipt"]["artifact"].clone())
            .expect("authoring receipt carries exact package ref");
    let store = skiff_deployment::storage::CanonicalArtifactStore::open(&artifact_root)
        .expect("open production artifact store");
    let package = store
        .read_package_artifact(&package_ref)
        .expect("read production package artifact");
    let bytecode_ref = package
        .bytecode
        .as_ref()
        .expect("production publication attaches bytecode");
    let bytecode = store
        .read_package_bytecode(&package_ref, bytecode_ref)
        .expect("production publication persists admitted bytecode");
    let function = bytecode
        .artifact()
        .image
        .functions
        .get("main::consume")
        .expect("production bytecode carries the source handler");
    let decoded = skiff_artifact_model::bytecode::BoundedDecoder::new()
        .decode_function(&function.words)
        .expect("published wordcode decodes");
    let parameter_shape_ref = function.frame_layout.parameter_slots[0]
        .dense_record_shape_ref
        .expect("rawHttp handler carries its exact request materialization layout");
    let BytecodePoolEntry::ShapeRef {
        shape: parameter_shape,
    } = &bytecode.artifact().image.pools.shapes[parameter_shape_ref as usize]
    else {
        panic!("rawHttp parameter layout selects a shape")
    };
    assert_eq!(
        parameter_shape
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["body", "headers", "method", "path", "query", "url"]
    );
    let mut emit_shapes = decoded
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, instruction)| instruction.descriptor.kind == Opcode::EmitStream)
        .map(|(ordinal, instruction)| {
            let resume_ref = instruction
                .descriptor
                .operand_word(OperandRole::ResumeRef, &instruction.operand_words)
                .expect("EmitStream carries a resume descriptor");
            let BytecodePoolEntry::ResumeDescriptor(resume) =
                &bytecode.artifact().image.pools.resume[resume_ref as usize]
            else {
                panic!("EmitStream resume operand selects a descriptor")
            };
            let shape_ref = resume
                .emit_stream_item_shape_ref
                .expect("each EmitStream carries its exact dense variant shape");
            let construct = decoded
                .instructions
                .get(ordinal.saturating_sub(1))
                .expect("EmitStream follows its exact construction");
            assert_eq!(construct.descriptor.kind, Opcode::NewRecord);
            assert_eq!(
                construct
                    .descriptor
                    .operand_word(OperandRole::ShapeRef, &construct.operand_words),
                Some(shape_ref),
                "resume fact must reuse the exact construction shape"
            );
            let BytecodePoolEntry::ShapeRef { shape } =
                &bytecode.artifact().image.pools.shapes[shape_ref as usize]
            else {
                panic!("EmitStream item shape ref selects a dense shape")
            };
            let names = shape
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>();
            if names == ["headers", "status", "tag"] {
                let carrier_types = shape
                    .fields
                    .iter()
                    .map(|field| {
                        let BytecodePoolEntry::TypeRef { ty, .. } =
                            &bytecode.artifact().image.pools.types[field.type_ref as usize]
                        else {
                            panic!("shape field selects a TypeRef row")
                        };
                        ty
                    })
                    .collect::<Vec<_>>();
                assert!(matches!(
                    carrier_types[0],
                    skiff_artifact_model::TypeRefIr::Builtin { name, args }
                        if name == "Array" && args.len() == 1
                ));
                assert_eq!(
                    carrier_types[1],
                    &skiff_artifact_model::TypeRefIr::builtin("number"),
                    "integer source semantics lower to the number VM carrier"
                );
                assert_eq!(
                    carrier_types[2],
                    &skiff_artifact_model::TypeRefIr::builtin("string"),
                    "literal tag source semantics lower to the string VM carrier"
                );
            }
            names
        })
        .collect::<Vec<_>>();
    emit_shapes.sort_unstable();
    assert_eq!(
        emit_shapes,
        [
            vec!["headers", "status", "tag"],
            vec!["tag"],
            vec!["tag", "value"],
        ],
        "start/chunk/end sites retain all distinct exact dense variant layouts"
    );
    assert!(decoded.instructions.windows(2).any(|pair| {
        pair[0].descriptor.kind == Opcode::TakeSlot
            && pair[1].descriptor.kind == Opcode::TakeDenseField
    }));
    assert_eq!(
        decoded
            .instructions
            .iter()
            .filter(|instruction| instruction.descriptor.kind == Opcode::NewArrayBuilder)
            .count(),
        1
    );
    assert_eq!(
        decoded
            .instructions
            .iter()
            .filter(|instruction| instruction.descriptor.kind == Opcode::FreezeArray)
            .count(),
        1
    );
    assert_eq!(
        function
            .relocations
            .iter()
            .filter(|relocation| {
                matches!(
                    relocation,
                    skiff_artifact_model::BytecodeRelocation::IntrinsicRef { intrinsic }
                        if matches!(
                            &intrinsic.target,
                            skiff_artifact_model::BytecodeIntrinsicRef::Static { .. }
                        )
                )
            })
            .count(),
        0
    );
    let deployment_ref: skiff_artifact_model::ServiceDeploymentRef =
        serde_json::from_value(receipt["serviceDeploymentReceipt"]["deployment"].clone())
            .expect("authoring receipt carries the formal deployment reference");
    let deployment = store
        .read_service_deployment(&deployment_ref)
        .expect("read the formally projected service deployment");
    let entry_key = skiff_artifact_model::GatewayEntryKey::parse("consume")
        .expect("fixture gateway key is canonical");
    let entry = deployment
        .gateway_entries
        .get(&entry_key)
        .expect("formal deployment carries the admitted gateway entry");
    assert_eq!(deployment.gateway_entries.len(), 1);
    assert_eq!(entry.handler.as_ref(), Some(&function.effect_summary_ref));
    assert_eq!(
        entry.gateway_entry_identity,
        skiff_artifact_identity::gateway_entry_identity(&entry.protocol_surface)
            .expect("formal gateway identity is derived from its exact typed surface")
    );
    let skiff_artifact_model::GatewayProtocolSurface::Http(surface) =
        &entry.protocol_surface.protocol
    else {
        panic!("formal raw HTTP entry must retain its HTTP protocol surface")
    };
    assert_eq!(
        surface.adapter_kind,
        skiff_artifact_model::GatewayAdapterKind::RawHttp
    );
    assert_eq!(
        surface.dispatch_mode,
        skiff_artifact_model::GatewayDispatchMode::ServerStream
    );
    assert_eq!(
        surface.external_sources,
        [skiff_artifact_model::GatewayAdapterSource::HttpRequest]
    );
    assert!(surface.request_body_schema.is_none());
    assert!(surface.response_schema.is_none());
    assert!(surface.stream_item_schema.is_some());
    std::fs::remove_dir_all(temp).expect("remove production fixture tree");
}

#[test]
fn production_authoring_publishes_direct_throw_and_rethrow_catch_discriminators() {
    let package_id = "test.skiff/diverging-catch-types";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, None);
    std::fs::write(
        package_root.join("main.skiff"),
        PRODUCTION_DIVERGING_CATCH_SOURCE,
    )
    .expect("write diverging catch source");

    let receipt = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect("direct throw and rethrow catch facts must cross production publication");
    let package_ref: skiff_artifact_model::PackageArtifactRef =
        serde_json::from_value(receipt["packageArtifactReceipt"]["artifact"].clone())
            .expect("authoring receipt carries package ref");
    let store = skiff_deployment::storage::CanonicalArtifactStore::open(&artifact_root)
        .expect("open production artifact store");
    let package = store
        .read_package_artifact(&package_ref)
        .expect("read published package artifact");
    let bytecode = store
        .read_package_bytecode(
            &package_ref,
            package
                .bytecode
                .as_ref()
                .expect("publication attaches admitted bytecode"),
        )
        .expect("read published bytecode");

    for function_key in ["main::direct", "main::rethrowing"] {
        let function = bytecode
            .artifact()
            .image
            .functions
            .get(function_key)
            .unwrap_or_else(|| panic!("published bytecode carries {function_key}"));
        let decoded = skiff_artifact_model::bytecode::BoundedDecoder::new()
            .decode_function(&function.words)
            .unwrap_or_else(|error| panic!("decode {function_key}: {error}"));
        assert!(decoded
            .instructions
            .iter()
            .any(|instruction| instruction.descriptor.kind == Opcode::GetDenseField));
        assert!(decoded
            .instructions
            .iter()
            .any(|instruction| instruction.descriptor.kind == Opcode::Equal));
    }
    assert!(bytecode.artifact().image.pools.types.iter().all(|entry| {
        !matches!(
            entry,
            BytecodePoolEntry::TypeRef {
                ty: skiff_artifact_model::TypeRefIr::Builtin { name, args },
                ..
            } if name == "unknown" && args.is_empty()
        )
    }));
    std::fs::remove_dir_all(temp).expect("remove diverging catch fixture tree");
}

#[test]
fn production_local_integer_catch_default_publishes_its_number_producer() {
    let package_id = "test.skiff/local-integer-catch-default";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, None);
    let source = PRODUCTION_DIVERGING_CATCH_SOURCE.replacen(
        "type LeafB {\n  marker: number,",
        "type LeafB {\n  marker: integer,",
        1,
    );
    assert_ne!(source, PRODUCTION_DIVERGING_CATCH_SOURCE);
    std::fs::write(package_root.join("main.skiff"), source)
        .expect("write local integer catch-default source");

    let receipt = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect("local integer catch default must publish from the actual zero producer");
    let package_ref: skiff_artifact_model::PackageArtifactRef =
        serde_json::from_value(receipt["packageArtifactReceipt"]["artifact"].clone())
            .expect("authoring receipt carries package ref");
    let store = skiff_deployment::storage::CanonicalArtifactStore::open(&artifact_root)
        .expect("open production artifact store");
    let package = store
        .read_package_artifact(&package_ref)
        .expect("read published package artifact");
    let bytecode = store
        .read_package_bytecode(
            &package_ref,
            package
                .bytecode
                .as_ref()
                .expect("publication attaches admitted bytecode"),
        )
        .expect("read published bytecode");

    let mut leaf_b_default_shapes = 0usize;
    for entry in &bytecode.artifact().image.pools.shapes {
        let BytecodePoolEntry::ShapeRef { shape } = entry else {
            continue;
        };
        let BytecodePoolEntry::TypeRef { ty: owner, .. } =
            &bytecode.artifact().image.pools.types[shape.type_ref as usize]
        else {
            continue;
        };
        if owner
            != &(skiff_artifact_model::TypeRefIr::PublicationType {
                module_path: "main".to_string(),
                type_index: 1,
            })
        {
            continue;
        }
        leaf_b_default_shapes += 1;
        let marker = shape
            .fields
            .iter()
            .find(|field| field.name == "marker")
            .expect("LeafB default shape retains marker");
        let BytecodePoolEntry::TypeRef { ty, .. } =
            &bytecode.artifact().image.pools.types[marker.type_ref as usize]
        else {
            panic!("LeafB marker selects one exact TypeRef row")
        };
        assert_eq!(
            ty,
            &skiff_artifact_model::TypeRefIr::builtin("number"),
            "the compiler-owned zero default is a Number producer even though LeafB.marker is semantically integer"
        );
    }
    assert!(
        leaf_b_default_shapes > 0,
        "the mismatch catch publishes its local LeafB default shape"
    );

    std::fs::remove_dir_all(temp).expect("remove local integer catch-default fixture tree");
}

#[test]
fn production_nested_record_array_writable_path_uses_exact_carrier_edges() {
    let package_id = "test.skiff/nested-writable-carriers";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, None);
    std::fs::write(
        package_root.join("main.skiff"),
        PRODUCTION_NESTED_WRITABLE_SOURCE,
    )
    .expect("write nested writable source");

    let receipt = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect("nested record/array writable path must publish exact carrier edges");
    let package_ref: skiff_artifact_model::PackageArtifactRef =
        serde_json::from_value(receipt["packageArtifactReceipt"]["artifact"].clone())
            .expect("authoring receipt carries package ref");
    let store = skiff_deployment::storage::CanonicalArtifactStore::open(&artifact_root)
        .expect("open production artifact store");
    let package = store
        .read_package_artifact(&package_ref)
        .expect("read published package artifact");
    let bytecode = store
        .read_package_bytecode(
            &package_ref,
            package
                .bytecode
                .as_ref()
                .expect("publication attaches admitted bytecode"),
        )
        .expect("read published bytecode");
    let artifact = bytecode.artifact();
    let function = artifact
        .image
        .functions
        .get("main::rewrite")
        .expect("published bytecode carries rewrite");
    let decoded = skiff_artifact_model::bytecode::BoundedDecoder::new()
        .decode_function(&function.words)
        .expect("nested writable wordcode decodes");
    let (write_ordinal, instruction) = decoded
        .instructions
        .iter()
        .enumerate()
        .find(|(_, instruction)| instruction.descriptor.kind == Opcode::SetWritablePath)
        .expect("nested assignment emits SetWritablePath");
    let path_ref = instruction
        .descriptor
        .operand_word(OperandRole::WritablePathRef, &instruction.operand_words)
        .expect("SetWritablePath carries its exact path ref");
    let BytecodePoolEntry::WritablePath(path) =
        &artifact.image.pools.writable_paths[path_ref as usize]
    else {
        panic!("writable operand selects a path declaration")
    };
    let (
        WritablePathSegment::DenseField { .. },
        WritablePathSegment::ArrayIndex {
            selector_ordinal: 0,
            element_type_ref,
        },
        WritablePathSegment::DenseField {
            shape_ref: cell_shape_ref,
            field_ordinal: count_ordinal,
        },
    ) = (&path.segments[0], &path.segments[1], &path.segments[2])
    else {
        panic!("nested write retains DenseField -> ArrayIndex -> DenseField facts")
    };
    assert_eq!(path.selector_count(), 1);
    let BytecodePoolEntry::TypeRef {
        ty: array_element, ..
    } = &artifact.image.pools.types[*element_type_ref as usize]
    else {
        panic!("Array path element selects one exact TypeRef row")
    };
    let BytecodePoolEntry::ShapeRef { shape: cell_shape } =
        &artifact.image.pools.shapes[*cell_shape_ref as usize]
    else {
        panic!("terminal Cell path selects one exact shape")
    };
    let BytecodePoolEntry::TypeRef { ty: cell_owner, .. } =
        &artifact.image.pools.types[cell_shape.type_ref as usize]
    else {
        panic!("Cell shape owner selects one exact TypeRef row")
    };
    assert_eq!(array_element, cell_owner);
    let count_field = &cell_shape.fields[*count_ordinal as usize];
    assert_eq!(count_field.name, "count");
    let BytecodePoolEntry::TypeRef {
        ty: writable_count, ..
    } = &artifact.image.pools.types[count_field.type_ref as usize]
    else {
        panic!("writable count field selects one exact TypeRef row")
    };
    assert_eq!(
        writable_count,
        &skiff_artifact_model::TypeRefIr::builtin("number")
    );
    let BytecodePoolEntry::TypeRef { ty: leaf, .. } =
        &artifact.image.pools.types[path.leaf_type_ref as usize]
    else {
        panic!("writable leaf selects one exact TypeRef row")
    };
    assert_eq!(leaf, &skiff_artifact_model::TypeRefIr::builtin("number"));

    let mut reachable_cell_shapes = 0usize;
    for entry in &artifact.image.pools.shapes {
        let BytecodePoolEntry::ShapeRef { shape } = entry else {
            continue;
        };
        let Some(count) = shape.fields.iter().find(|field| field.name == "count") else {
            continue;
        };
        reachable_cell_shapes += 1;
        let BytecodePoolEntry::TypeRef { ty, .. } =
            &artifact.image.pools.types[count.type_ref as usize]
        else {
            panic!("reachable Cell count field selects one exact TypeRef row")
        };
        assert_eq!(
            ty,
            &skiff_artifact_model::TypeRefIr::builtin("number"),
            "every value-local Cell shape must retain the source Number producer"
        );
    }
    assert!(reachable_cell_shapes > 0);

    let mut read_back_count = 0usize;
    for instruction in decoded.instructions.iter().skip(write_ordinal + 1) {
        if instruction.descriptor.kind != Opcode::GetDenseField {
            continue;
        }
        let shape_ref = instruction
            .descriptor
            .operand_word(OperandRole::ShapeRef, &instruction.operand_words)
            .expect("GetDenseField carries one exact shape");
        let field_ordinal = instruction
            .descriptor
            .operand_word(OperandRole::FieldOrdinal, &instruction.operand_words)
            .expect("GetDenseField carries one exact field ordinal");
        let BytecodePoolEntry::ShapeRef { shape } =
            &artifact.image.pools.shapes[shape_ref as usize]
        else {
            panic!("GetDenseField selects one exact shape")
        };
        let field = &shape.fields[field_ordinal as usize];
        if field.name != "count" {
            continue;
        }
        read_back_count += 1;
        let BytecodePoolEntry::TypeRef { ty, .. } =
            &artifact.image.pools.types[field.type_ref as usize]
        else {
            panic!("read-back count field selects one exact TypeRef row")
        };
        assert_eq!(
            ty,
            &skiff_artifact_model::TypeRefIr::builtin("number"),
            "post-write GetDenseField must reuse the Array element producer shape"
        );
    }
    assert!(
        read_back_count > 0,
        "fixture reads count after the nested write"
    );

    std::fs::remove_dir_all(temp).expect("remove nested writable fixture tree");
}

#[test]
fn production_nested_writable_rejects_host_integer_source_number_conflict() {
    let package_id = "test.skiff/nested-writable-carrier-conflict";
    let http = "consume:\n  method: POST\n  path: /phase-5/nested-conflict\n  kind: rawHttp\n  handler: main.consume\n  adapterArgs:\n    - param: request\n      source: { kind: http.request }\n";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, Some(http));
    std::fs::write(
        package_root.join("main.skiff"),
        r#"import std

type Cell { count: integer }
type State { rows: Array<Cell> }

function consume(request: std.http.HttpRequest) -> Stream<std.http.HttpResponseStreamEvent> {
  final outbound = std.http.HttpClientRequest {
    method: request.method,
    url: request.body.toUtf8String(),
    headers: request.headers,
    body: null,
    timeoutMs: null,
  }
  final unary = std.http.request(outbound)
  var state = State { rows: [Cell { count: 0 }] }
  state.rows[0].count = unary.status
  emit({ tag: "start", status: 207, headers: unary.headers })
  emit({ tag: "chunk", value: unary.body })
  emit({ tag: "end" })
  return null
}
"#,
    )
    .expect("write conflicting writable source");

    let error = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect_err("Integer and Number producers cannot share one exact writable leaf");
    let service_typed = error.downcast_ref::<crate::ServicePackageCompileError>();
    let package_typed = error.downcast_ref::<PackageCompileError>();
    assert!(
        service_typed.is_some_and(|typed| matches!(
            typed,
            crate::ServicePackageCompileError::Package(PackageCompileError::BytecodeEmission {
                source: skiff_compiler_emission::BytecodeEmissionError::UnsupportedConstruct {
                    construct: "exact machine carrier facts",
                    ..
                }
            })
        )) || package_typed.is_some_and(|typed| matches!(
            typed,
            PackageCompileError::BytecodeEmission {
                source: skiff_compiler_emission::BytecodeEmissionError::UnsupportedConstruct {
                    construct: "exact machine carrier facts",
                    ..
                }
            }
        )),
        "expected exact host-Integer/source-Number carrier rejection, got: {error:?}"
    );
    assert_no_publication_pointers(&artifact_root, package_id);
    std::fs::remove_dir_all(temp).expect("remove conflicting writable fixture tree");
}

#[test]
fn actor_self_field_emits_exact_self_root_facts() {
    let package_id = "test.skiff/actor-self-field-emission";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, None);
    std::fs::write(
        package_root.join("main.skiff"),
        r#"
type Counter {
  id: string,
  count: number,
}

actor Counter {
  key(id)
  create()
}

impl Counter {
  function create() -> void {
    self.count = 0
  }

  function increment() -> number {
    self.count = self.count + 1
    return self.count
  }
}
"#,
    )
    .expect("write actor self-field fixture");
    let receipt = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect("actor self-field package publishes bytecode");
    let package_ref: skiff_artifact_model::PackageArtifactRef =
        serde_json::from_value(receipt["packageArtifactReceipt"]["artifact"].clone())
            .expect("authoring receipt carries package ref");
    let store = skiff_deployment::storage::CanonicalArtifactStore::open(&artifact_root)
        .expect("open actor artifact store");
    let package = store
        .read_package_artifact(&package_ref)
        .expect("read published actor package");
    let bytecode = store
        .read_package_bytecode(
            &package_ref,
            package
                .bytecode
                .as_ref()
                .expect("actor publication attaches admitted bytecode"),
        )
        .expect("read published actor bytecode");
    let artifact = bytecode.artifact();
    for function_key in ["main::Counter.create", "main::Counter.increment"] {
        let function = artifact
            .image
            .functions
            .get(function_key)
            .unwrap_or_else(|| panic!("published bytecode carries {function_key}"));
        let decoded = skiff_artifact_model::bytecode::BoundedDecoder::new()
            .decode_function(&function.words)
            .expect("actor self-field wordcode decodes");
        let write = decoded
            .instructions
            .iter()
            .find(|instruction| instruction.descriptor.kind == Opcode::SetWritablePath)
            .unwrap_or_else(|| panic!("{function_key} emits ActorSelfField SetWritablePath"));
        let path_ref = write
            .descriptor
            .operand_word(OperandRole::WritablePathRef, &write.operand_words)
            .expect("ActorSelfField SetWritablePath carries its exact path ref");
        let BytecodePoolEntry::WritablePath(path) =
            &artifact.image.pools.writable_paths[path_ref as usize]
        else {
            panic!("ActorSelfField writable operand selects a path declaration")
        };
        let [WritablePathSegment::DenseField {
            shape_ref,
            field_ordinal,
        }] = path.segments.as_slice()
        else {
            panic!("ActorSelfField root retains one exact dense field segment")
        };
        let BytecodePoolEntry::ShapeRef { shape } =
            &artifact.image.pools.shapes[*shape_ref as usize]
        else {
            panic!("ActorSelfField path selects one exact shape")
        };
        let field = &shape.fields[*field_ordinal as usize];
        assert_eq!(field.name, "count");
        let BytecodePoolEntry::TypeRef { ty, .. } =
            &artifact.image.pools.types[field.type_ref as usize]
        else {
            panic!("ActorSelfField count field selects one exact TypeRef row")
        };
        assert_eq!(ty, &skiff_artifact_model::TypeRefIr::builtin("number"));
    }
    let increment = artifact
        .image
        .functions
        .get("main::Counter.increment")
        .expect("published bytecode carries increment");
    let decoded = skiff_artifact_model::bytecode::BoundedDecoder::new()
        .decode_function(&increment.words)
        .expect("actor increment wordcode decodes");
    assert!(decoded
        .instructions
        .iter()
        .any(|instruction| instruction.descriptor.kind == Opcode::LoadSlot));
    assert!(decoded
        .instructions
        .iter()
        .any(|instruction| instruction.descriptor.kind == Opcode::GetDenseField));
    std::fs::remove_dir_all(temp).expect("remove actor self-field fixture tree");
}

#[test]
fn callback_schema_closure_keeps_provider_owner_and_record() {
    let record = skiff_artifact_model::PackageSchemaTypeRecord {
        package_id: "example.com/phase-6-callback-provider".to_string(),
        stable_schema_key: "Handler".to_string(),
        package_schema_type_id: skiff_artifact_model::derive_package_schema_type_id(
            "example.com/phase-6-callback-provider",
            "Handler",
            &skiff_artifact_model::PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: skiff_artifact_model::ContractTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            },
        )
        .expect("callback provider schema identity derives"),
        canonical_descriptor: skiff_artifact_model::PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: skiff_artifact_model::ContractTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
        },
    };
    let mut artifact = bytecode_artifact_fixture();
    artifact.image.pools.types = vec![skiff_artifact_model::BytecodePoolEntry::TypeRef {
        ty: skiff_artifact_model::TypeRefIr::PackageSchema {
            package_id: record.package_id.clone(),
            stable_schema_key: record.stable_schema_key.clone(),
            package_schema_type_id: record.package_schema_type_id.clone(),
        },
        representation_carrier: None,
        plan: skiff_artifact_model::ValueTransferPlan::SnapshotShare {
            drop: skiff_artifact_model::ValueDropPlan::Trivial,
        },
    }];
    let available = BTreeMap::from([(record.package_schema_type_id.clone(), record.clone())]);
    let facts = skiff_compiler_emission::bytecode::collect_bytecode_schema_facts(
        "test.skiff/callback-consumer",
        &artifact,
        &available,
    )
    .expect("foreign callback schema closure derives");
    assert!(facts.records.is_empty());
    assert!(facts
        .referenced_package_ids
        .contains("example.com/phase-6-callback-provider"));
    let owner_facts = skiff_compiler_emission::bytecode::collect_bytecode_schema_facts(
        "example.com/phase-6-callback-provider",
        &artifact,
        &available,
    )
    .expect("provider callback schema closure derives");
    assert_eq!(
        owner_facts.records.get(&record.package_schema_type_id),
        Some(&record)
    );
}

#[test]
fn production_multi_carrier_union_frame_fails_before_publication() {
    let package_id = "test.skiff/multi-carrier-union-frame";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, None);
    std::fs::write(
        package_root.join("main.skiff"),
        r#"
type LeafA { marker: number }
type LeafB { marker: number }

function consume(value: LeafA | LeafB) -> void {
  throw value
}

function run(seed: number) -> void {
  final value: LeafA | LeafB = LeafA { marker: seed }
  consume(value)
}
"#,
    )
    .expect("write multi-carrier union fixture");

    let error = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect_err("a union frame without one exact physical carrier must fail closed");
    let service_typed = error.downcast_ref::<crate::ServicePackageCompileError>();
    let package_typed = error.downcast_ref::<PackageCompileError>();
    assert!(
        service_typed.is_some_and(|typed| matches!(
            typed,
            crate::ServicePackageCompileError::Package(PackageCompileError::BytecodeEmission {
                source: skiff_compiler_emission::BytecodeEmissionError::UnsupportedConstruct {
                    construct: "exact machine carrier facts",
                    ..
                }
            })
        )) || package_typed.is_some_and(|typed| matches!(
            typed,
            PackageCompileError::BytecodeEmission {
                source: skiff_compiler_emission::BytecodeEmissionError::UnsupportedConstruct {
                    construct: "exact machine carrier facts",
                    ..
                }
            }
        )),
        "expected exact machine-carrier rejection, got: {error:?}"
    );
    assert_no_publication_pointers(&artifact_root, package_id);
    std::fs::remove_dir_all(temp).expect("remove multi-carrier union fixture tree");
}

#[test]
fn production_raw_http_parameter_shape_does_not_depend_on_field_access() {
    const SOURCE: &str = r#"import std

function consume(
  request: std.http.HttpRequest
) -> Stream<std.http.HttpResponseStreamEvent> {
  emit({ tag: "end" })
  return null
}
"#;
    let package_id = "test.skiff/c5-shapeless-request";
    let http = "consume:\n  method: POST\n  path: /phase-5/shapeless\n  kind: rawHttp\n  handler: main.consume\n  adapterArgs:\n    - param: request\n      source: { kind: http.request }\n";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, Some(http));
    std::fs::write(package_root.join("main.skiff"), SOURCE).expect("write shapeless source");
    let receipt = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect("unused canonical HttpRequest still emits its exact layout fact");
    let package_ref: skiff_artifact_model::PackageArtifactRef =
        serde_json::from_value(receipt["packageArtifactReceipt"]["artifact"].clone())
            .expect("authoring receipt carries package ref");
    let store = skiff_deployment::storage::CanonicalArtifactStore::open(&artifact_root)
        .expect("open production artifact store");
    let package = store
        .read_package_artifact(&package_ref)
        .expect("read production package artifact");
    let bytecode = store
        .read_package_bytecode(&package_ref, package.bytecode.as_ref().unwrap())
        .expect("read production bytecode");
    let function = &bytecode.artifact().image.functions["main::consume"];
    let decoded = skiff_artifact_model::bytecode::BoundedDecoder::new()
        .decode_function(&function.words)
        .expect("decode handler");
    assert!(decoded.instructions.iter().all(|instruction| {
        !matches!(
            instruction.descriptor.kind,
            Opcode::GetDenseField | Opcode::TakeDenseField
        )
    }));
    let shape_ref = function.frame_layout.parameter_slots[0]
        .dense_record_shape_ref
        .expect("compiler emits the root shape without field opcodes");
    let BytecodePoolEntry::ShapeRef { shape } =
        &bytecode.artifact().image.pools.shapes[shape_ref as usize]
    else {
        panic!("parameter fact selects a dense shape")
    };
    assert_eq!(
        shape
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["body", "headers", "method", "path", "query", "url"]
    );
    std::fs::remove_dir_all(temp).expect("remove production fixture tree");
}

#[test]
fn production_gateway_negatives_fail_before_package_or_release_pointer() {
    for (label, http, expected_field) in [
        (
            "wrong-kind",
            "consume:\n  method: POST\n  path: /phase-5/compiler\n  kind: typedJson\n  handler: main.consume\n  adapterArgs:\n    - param: request\n      source: { kind: http.body }\n",
            "handler",
        ),
        (
            "wrong-handler",
            "consume:\n  method: POST\n  path: /phase-5/compiler\n  kind: rawHttp\n  handler: main.missing\n  adapterArgs:\n    - param: request\n      source: { kind: http.request }\n",
            "handler",
        ),
    ] {
        let package_id = format!("test.skiff/c5-production-{label}");
        let (platform, package_root, artifact_root, temp) =
            production_fixture(&package_id, Some(http));
        let error = crate::authoring::build_authoring_object(
            &platform,
            crate::authoring::AuthoringObject::Package,
            &package_root,
            &artifact_root,
            "skiff-test",
            true,
        )
        .expect_err("invalid canonical gateway input must fail before publication");
        let typed = error
            .downcast_ref::<crate::ServicePackageCompileError>()
            .expect("gateway projection preserves its typed service compile error");
        assert!(matches!(
            typed,
            crate::ServicePackageCompileError::HttpGateway(
                crate::http_gateway_projection::HttpGatewayProjectionError::InvalidEntry {
                    field,
                    ..
                }
            ) if *field == expected_field
        ));
        assert_no_publication_pointers(&artifact_root, &package_id);
        std::fs::remove_dir_all(temp).expect("remove negative fixture tree");
    }
}

#[test]
fn production_stream_shape_mismatches_fail_typed_before_publication() {
    const HTTP: &str = "consume:\n  method: POST\n  path: /phase-5/compiler\n  kind: rawHttp\n  handler: main.consume\n  adapterArgs:\n    - param: request\n      source: { kind: http.request }\n";
    let start = "  emit({\n    tag: \"start\",\n    status: 207,\n    headers: [],\n  })";
    for (label, source, expected) in [
        (
            "nominal-mismatch",
            format!(
                "type WrongEvent {{ tag: string }}\n\n{}",
                PRODUCTION_SERVER_STREAM_SOURCE.replacen(
                    start,
                    "  emit(WrongEvent { tag: \"start\" })",
                    1,
                )
            ),
            "emit chunk type mismatch",
        ),
        (
            "field-set-mismatch",
            PRODUCTION_SERVER_STREAM_SOURCE.replacen("    headers: [],\n", "", 1),
            "missing required object literal field `headers`",
        ),
    ] {
        let package_id = format!("test.skiff/c5-production-{label}");
        let (platform, package_root, artifact_root, temp) =
            production_fixture(&package_id, Some(HTTP));
        std::fs::write(package_root.join("main.skiff"), source)
            .expect("write invalid stream shape source");
        let error = crate::authoring::build_authoring_object(
            &platform,
            crate::authoring::AuthoringObject::Package,
            &package_root,
            &artifact_root,
            "skiff-test",
            true,
        )
        .expect_err("invalid stream item shape must fail before publication");
        let typed = error
            .downcast_ref::<crate::ServicePackageCompileError>()
            .expect("source rejection preserves its typed service compile error");
        assert!(
            matches!(
                typed,
                crate::ServicePackageCompileError::Package(
                    PackageCompileError::ContractValidation { message }
                ) if message.contains(expected)
            ),
            "expected diagnostic {expected:?}, got: {typed}"
        );
        assert_no_publication_pointers(&artifact_root, &package_id);
        std::fs::remove_dir_all(temp).expect("remove invalid stream shape fixture tree");
    }
}

#[test]
fn ordinary_package_stream_shape_has_no_gateway_authority() {
    let package_id = "test.skiff/c5-ordinary-stream-shape";
    let (platform, package_root, artifact_root, temp) = production_fixture(package_id, None);
    let error = crate::authoring::build_authoring_object(
        &platform,
        crate::authoring::AuthoringObject::Package,
        &package_root,
        &artifact_root,
        "skiff-test",
        true,
    )
    .expect_err("a Stream nominal/type shape without gateway authority must fail closed");
    let typed = error
        .downcast_ref::<PackageCompileError>()
        .expect("ordinary package preserves its typed compiler rejection");
    assert!(matches!(
        typed,
        PackageCompileError::BytecodeEmission {
            source: skiff_compiler_emission::BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: skiff_compiler_emission::Phase1UnsupportedCapability::Stream,
                ..
            }
        }
    ));
    assert_no_publication_pointers(&artifact_root, package_id);
    std::fs::remove_dir_all(temp).expect("remove ordinary fixture tree");
}

fn production_fixture(
    package_id: &str,
    http: Option<&str>,
) -> (
    crate::CompilerPlatformSources,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let repository_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler manifest has repository root")
        .to_path_buf();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock follows Unix epoch")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!(
        "skiff-c5-production-negative-{}-{nonce}",
        std::process::id()
    ));
    let package_root = temp.join("package");
    let artifact_root = temp.join("artifacts");
    std::fs::create_dir_all(&package_root).expect("create production fixture root");
    std::fs::write(
        package_root.join("package.yml"),
        format!("id: {package_id}\nversion: 1.0.0\n"),
    )
    .expect("write package manifest");
    std::fs::write(package_root.join("api.yml"), "{}\n").expect("write API document");
    std::fs::write(
        package_root.join("main.skiff"),
        PRODUCTION_SERVER_STREAM_SOURCE,
    )
    .expect("write source fixture");
    if let Some(http) = http {
        std::fs::write(
            package_root.join("service.yml"),
            format!("id: {package_id}\n"),
        )
        .expect("write service manifest");
        std::fs::write(package_root.join("http.yml"), http).expect("write HTTP gateway document");
    }
    let platform = crate::CompilerPlatformSources::new(&repository_root)
        .expect("open production platform sources");
    crate::authoring::seed_official_std_package(&platform, &artifact_root)
        .expect("seed exact compiler-owned std package");
    (platform, package_root, artifact_root, temp)
}

fn assert_no_publication_pointers(artifact_root: &std::path::Path, package_id: &str) {
    let store = skiff_deployment::storage::CanonicalArtifactStore::open(artifact_root)
        .expect("open negative fixture store");
    assert!(store
        .read_package_artifact_pointer(package_id, "1.0.0")
        .expect("read package pointer")
        .is_none());
    assert!(store
        .read_release_pointer("skiff-test", package_id, "1.0.0")
        .expect("read release pointer")
        .is_none());
}

#[test]
fn disabled_lane_rejects_a_valid_historical_package_registry() {
    let mut projected = projected_fixture(PACKAGE_ID);
    projected.artifact.platform_error_projection_registry = historical_registry_fixture();
    assign_package_artifact_identities(&mut projected.artifact).unwrap();
    validate_platform_error_projection_registry_ref_shape(
        &projected.artifact.platform_error_projection_registry,
    )
    .unwrap();
    assert!(validate_current_platform_error_projection_registry_ref(
        &projected.artifact.platform_error_projection_registry,
    )
    .is_err());
    let source = projected.artifact.clone();

    let error = attach_bytecode_execution(&projected, &PackageBytecodeLane::Disabled)
        .unwrap_err()
        .to_string();

    assert!(error.contains("platform error projection registry mismatch"));
    assert_eq!(projected.artifact, source);
}

#[test]
fn enabled_lane_rejects_package_registry_different_from_admitted_bytecode() {
    let mut projected = projected_fixture(PACKAGE_ID);
    let lane = enabled_lane(PACKAGE_ID);
    let admitted_registry = lane
        .receipt()
        .unwrap()
        .authorities()
        .platform_error_projection_registry();
    projected.artifact.platform_error_projection_registry = historical_registry_fixture();
    assign_package_artifact_identities(&mut projected.artifact).unwrap();
    assert_ne!(
        &projected.artifact.platform_error_projection_registry,
        admitted_registry
    );
    let source = projected.artifact.clone();

    let error = attach_bytecode_execution(&projected, &lane)
        .unwrap_err()
        .to_string();

    assert!(error.contains("platform error projection registry mismatch"));
    assert!(error.contains("admitted bytecode handoff"));
    assert_eq!(projected.artifact, source);
}

#[test]
fn handoff_package_mismatch_fails_after_attachment_without_mutating_source() {
    let projected = projected_fixture(PACKAGE_ID);
    let source = projected.artifact.clone();
    let lane = enabled_lane("example.com/other-bytecode-owner");

    let error = attach_bytecode_execution(&projected, &lane)
        .unwrap_err()
        .to_string();

    assert!(error.contains("does not match admitted statement manifest package id"));
    assert_eq!(projected.artifact, source);
}

#[test]
fn half_states_fail_without_mutating_the_input_projection() {
    let lane = enabled_lane(PACKAGE_ID);
    let handoff = lane.handoff().unwrap();

    let mut bytecode_only = projected_fixture(PACKAGE_ID);
    bytecode_only.artifact.bytecode = Some(handoff.reference().clone());
    let bytecode_only_source = bytecode_only.artifact.clone();
    let error = attach_bytecode_execution(&bytecode_only, &lane)
        .unwrap_err()
        .to_string();
    assert!(error.contains("exact canonical empty statement manifest"));
    assert_eq!(bytecode_only.artifact, bytecode_only_source);

    let mut manifest_only = projected_fixture(PACKAGE_ID);
    manifest_only.artifact.bytecode_statement_manifest_identity =
        handoff.statement_manifest_receipt().identity().clone();
    let manifest_only_source = manifest_only.artifact.clone();
    let error = attach_bytecode_execution(&manifest_only, &lane)
        .unwrap_err()
        .to_string();
    assert!(error.contains("exact canonical empty statement manifest"));
    assert_eq!(manifest_only.artifact, manifest_only_source);

    let disabled_error = attach_bytecode_execution(&manifest_only, &PackageBytecodeLane::Disabled)
        .unwrap_err()
        .to_string();
    assert!(disabled_error.contains("package-specific canonical empty manifest"));
    assert_eq!(manifest_only.artifact, manifest_only_source);
}

#[test]
fn final_package_validation_rejects_manifest_drift() {
    let projected = projected_fixture(PACKAGE_ID);
    let lane = enabled_lane(PACKAGE_ID);
    let attached = attach_bytecode_execution(&projected, &lane).unwrap();
    let mut published = publish_projected_package_artifact(&attached, &[]).unwrap();
    published.artifact.bytecode_statement_manifest_identity =
        derive_bytecode_statement_manifest_identity("example.com/other-package", &[]).unwrap();

    let error = PackageCompileOutput::try_new(published, lane, Default::default())
        .unwrap_err()
        .to_string();

    assert!(error.contains("statement manifest"));
    assert!(error.contains("does not exactly match"));
}

fn enabled_lane(package_id: &str) -> PackageBytecodeLane {
    let artifact = bytecode_artifact_fixture();
    let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    let statement_manifest = independent_statement_manifest_fixture();
    let manifest_identity =
        derive_bytecode_statement_manifest_identity(package_id, &statement_manifest).unwrap();
    let handoff = BytecodeCompilationHandoff::try_new(
        package_id.to_string(),
        statement_manifest,
        manifest_identity,
        artifact,
        reference,
    )
    .unwrap();
    PackageBytecodeLane::Enabled(Box::new(handoff))
}

fn bytecode_artifact_fixture() -> BytecodeArtifact {
    let event_function = function_fixture("module::event", 0, statement_entries_fixture());
    let zero_event_function = function_fixture("module::zero", 1, Vec::new());
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: skiff_artifact_model::opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        platform_error_projection_registry: current_platform_error_projection_registry_ref()
            .clone(),
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::from([
                (event_function.function_key.clone(), event_function),
                (
                    zero_event_function.function_key.clone(),
                    zero_event_function,
                ),
            ]),
            pools: BytecodePools::default(),
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    };
    assign_bytecode_identity(&mut artifact).unwrap();
    artifact
}

fn independent_statement_manifest_fixture() -> Vec<BytecodeFunctionStatementManifest> {
    // This fixture is authored from explicit source-event facts and never
    // inspects `BytecodeArtifact.image.functions` or its statement rows.
    vec![
        BytecodeFunctionStatementManifest::new(origin_fixture(0), statement_entries_fixture()),
        BytecodeFunctionStatementManifest::new(origin_fixture(1), Vec::new()),
    ]
}

fn function_fixture(
    function_key: &str,
    executable_index: u32,
    statement_entries: Vec<StatementEntry>,
) -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: function_key.to_string(),
        origin: origin_fixture(executable_index),
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![u32::from(descriptor_for_opcode(Opcode::Return).opcode)],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 0,
            slot_type_refs: Vec::new(),
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
            slot_plans: Vec::new(),
        },
        max_operand_depth: 0,
        effect_summary_ref: PackageCallableId::new(format!("operation:module:{executable_index}")),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries,
        source_map: Vec::new(),
    }
}

fn origin_fixture(executable_index: u32) -> BytecodeFunctionOrigin {
    BytecodeFunctionOrigin::Executable {
        executable: PackageExecutableCoordinate {
            file_ir_identity: format!("{FILE_IR_IDENTITY_PREFIX}:{}", "a".repeat(64)),
            module_path: "module".to_string(),
            executable_index,
        },
    }
}

fn statement_entries_fixture() -> Vec<StatementEntry> {
    vec![
        StatementEntry {
            pc: 0,
            sequence_ordinal: 0,
            attribution_id: StatementAttributionId::Statement {
                statement_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site_fixture(1),
        },
        StatementEntry {
            pc: 0,
            sequence_ordinal: 1,
            attribution_id: StatementAttributionId::Expression {
                expression_index: 0,
                occurrence_ordinal: 0,
            },
            site: source_site_fixture(2),
        },
    ]
}

fn source_site_fixture(source_id: u64) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    }
}

fn projected_fixture(package_id: &str) -> ProjectedPackageArtifact {
    let schema_types = BTreeMap::new();
    let schema_identity = package_schema_index_identity(package_id, &schema_types).unwrap();
    let package_schema_index = PackageSchemaIndex {
        package_id: package_id.to_string(),
        package_schema_index_identity: schema_identity.clone(),
        types: schema_types,
    };
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        platform_error_projection_registry: current_platform_error_projection_registry_ref()
            .clone(),
        files: Vec::new(),
        static_resources: Vec::new(),
        bytecode: None,
        bytecode_statement_manifest_identity: derive_bytecode_statement_manifest_identity(
            package_id,
            &[],
        )
        .unwrap(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: schema_identity,
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    assign_package_artifact_identities(&mut artifact).unwrap();
    ProjectedPackageArtifact {
        artifact,
        package_schema_index,
        package_schema_type_records: BTreeMap::new(),
        resolved_package_schema_type_records: BTreeMap::new(),
        file_ir_units: Vec::new(),
        resources: Vec::new(),
    }
}

fn historical_registry_fixture() -> PlatformErrorProjectionRegistryRef {
    let current = current_platform_error_projection_registry_ref();
    let historical_fingerprint = format!(
        "sha256:{}",
        if current.fingerprint().ends_with('0') {
            "1".repeat(64)
        } else {
            "0".repeat(64)
        }
    );
    serde_json::from_value(serde_json::json!({
        "registryId": current.registry_id(),
        "registryVersion": current.registry_version(),
        "fingerprint": historical_fingerprint,
    }))
    .unwrap()
}

#[test]
fn phase_2_bytecode_admission_source_facts_export_local_and_publication_nominals() {
    let record = skiff_artifact_model::TypeDescriptorIr::Record {
        fields: BTreeMap::from([(
            "name".to_string(),
            skiff_artifact_model::TypeRefIr::builtin("string"),
        )]),
    };
    let unit = skiff_compiler_lowering::mir::MirUnit {
        file_ir_identity: "file:facts".to_string(),
        package_id: "test.package".to_string(),
        module_path: "facts".to_string(),
        actor_declarations: Vec::new(),
        external_refs: skiff_artifact_model::ExternalRefTable::default(),
        source_map: skiff_artifact_model::SourceMapDto {
            format: String::new(),
            sources: Vec::new(),
            spans: Vec::new(),
        },
        type_table: vec![skiff_artifact_model::TypeDeclIr {
            name: "Person".to_string(),
            descriptor: record.clone(),
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        }],
        package_type_records: BTreeMap::new(),
        link_targets: skiff_artifact_model::FileLinkTargets::default(),
        constants: Vec::new(),
        functions: Vec::new(),
    };
    let facts = source_value_transfer_facts_for_units(&[unit]);
    let expected = skiff_compiler_source::SourceValueTransferNominalFact {
        declaration_module: "facts".to_string(),
        type_parameters: Vec::new(),
        semantics: skiff_compiler_source::SourceValueTransferNominalSemantics::Ordinary(record),
    };
    assert_eq!(
        facts.nominal(
            &skiff_compiler_source::SourceValueTransferNominalId::Local {
                module_path: "facts".to_string(),
                type_index: 0,
            }
        ),
        Some(&expected)
    );
    assert_eq!(
        facts.nominal(
            &skiff_compiler_source::SourceValueTransferNominalId::Publication {
                module_path: "facts".to_string(),
                type_index: 0,
            }
        ),
        Some(&expected)
    );
}

#[test]
fn phase_4_source_facts_do_not_synthesize_duration_alias_semantics() {
    let unit = package_fact_unit(
        skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string(),
        },
        "std.time.Duration",
        None,
        BTreeMap::new(),
    );
    let facts = source_value_transfer_facts_for_units(&[unit]);
    let identity = skiff_compiler_source::SourceValueTransferNominalId::PackageSymbol {
        package: skiff_compiler_source::SourceValueTransferPackageRef::PackageId(
            "skiff.run/std".to_string(),
        ),
        symbol_path: "std.time.Duration".to_string(),
        abi_expectation: None,
    };
    assert!(
        facts.nominal(&identity).is_none(),
        "Duration lifecycle must come only from an admitted materializer fact"
    );
}

#[test]
fn phase_5_source_facts_require_exact_canonical_package_record_authority() {
    const ABI: &str = "sha256:exact-http-abi";
    const PATH: &str = "std.http.HttpClientRequest";
    let fields = BTreeMap::from([(
        "url".to_string(),
        skiff_artifact_model::TypeRefIr::builtin("string"),
    )]);
    let exact = package_fact_unit(
        skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string(),
        },
        PATH,
        Some(ABI),
        fields.clone(),
    );
    let facts = source_value_transfer_facts_for_units(std::slice::from_ref(&exact));
    let identity = skiff_compiler_source::SourceValueTransferNominalId::PackageSymbol {
        package: skiff_compiler_source::SourceValueTransferPackageRef::PackageId(
            "skiff.run/std".to_string(),
        ),
        symbol_path: PATH.to_string(),
        abi_expectation: Some(ABI.to_string()),
    };
    assert_eq!(
        facts.nominal(&identity),
        Some(&skiff_compiler_source::SourceValueTransferNominalFact {
            declaration_module: "std.http".to_string(),
            type_parameters: Vec::new(),
            semantics: skiff_compiler_source::SourceValueTransferNominalSemantics::Ordinary(
                skiff_artifact_model::TypeDescriptorIr::Record {
                    fields: fields.clone(),
                },
            ),
        })
    );

    let mut duplicate = exact.clone();
    duplicate
        .external_refs
        .package_symbols
        .push(duplicate.external_refs.package_symbols[0].clone());
    assert!(source_value_transfer_facts_for_units(&[duplicate])
        .nominal(&identity)
        .is_none());

    for unowned in [
        package_fact_unit(
            skiff_artifact_model::PackageRefIr::Dependency {
                dependency_ref: "std".to_string(),
            },
            PATH,
            Some(ABI),
            fields.clone(),
        ),
        package_fact_unit(
            skiff_artifact_model::PackageRefIr::PackageId {
                package_id: "skiff.run/std".to_string(),
            },
            PATH,
            None,
            fields.clone(),
        ),
    ] {
        assert!(source_value_transfer_facts_for_units(&[unowned])
            .nominal(&identity)
            .is_none());
    }

    let drifted = package_fact_unit(
        skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string(),
        },
        PATH,
        Some(ABI),
        BTreeMap::from([(
            "url".to_string(),
            skiff_artifact_model::TypeRefIr::builtin("bytes"),
        )]),
    );
    assert!(source_value_transfer_facts_for_units(&[exact, drifted])
        .nominal(&identity)
        .is_none());
}

#[test]
fn phase_5_source_facts_own_the_canonical_http_boundary_lifecycle() {
    const ABI: &str = "sha256:exact-http-abi";
    let path = skiff_artifact_model::http_boundary::HTTP_RESPONSE_STREAM_EVENT_TYPE;
    let mut unit = package_fact_unit(
        skiff_artifact_model::PackageRefIr::PackageId {
            package_id: "skiff.run/std".to_string(),
        },
        path,
        Some(ABI),
        BTreeMap::new(),
    );
    unit.package_type_records.clear();

    let facts = source_value_transfer_facts_for_units(&[unit]);
    let ty = skiff_artifact_model::TypeRefIr::PackageSymbol {
        symbol: skiff_artifact_model::PackageSymbolRef {
            package: skiff_artifact_model::PackageRefIr::PackageId {
                package_id: "skiff.run/std".to_string(),
            },
            symbol_path: path.to_string(),
            abi_expectation: Some(ABI.to_string()),
        },
    };
    let plan = skiff_compiler_source::source_value_transfer_plan(
        &facts,
        skiff_compiler_source::SourceValueTransferPlanInput::concrete("main", &ty),
    )
    .expect("the compiler owns the canonical HTTP boundary lifecycle");
    assert_eq!(
        plan,
        skiff_artifact_model::ValueTransferPlan::SnapshotShare {
            drop: skiff_artifact_model::ValueDropPlan::SnapshotRelease,
        }
    );
}

fn package_fact_unit(
    package: skiff_artifact_model::PackageRefIr,
    symbol_path: &str,
    abi: Option<&str>,
    fields: BTreeMap<String, skiff_artifact_model::TypeRefIr>,
) -> skiff_compiler_lowering::mir::MirUnit {
    let owner = match &package {
        skiff_artifact_model::PackageRefIr::PackageId { package_id } => package_id.clone(),
        skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => dependency_ref.clone(),
    };
    skiff_compiler_lowering::mir::MirUnit {
        file_ir_identity: format!("file:{owner}"),
        package_id: "test.package".to_string(),
        module_path: "main".to_string(),
        actor_declarations: Vec::new(),
        external_refs: skiff_artifact_model::ExternalRefTable {
            package_symbols: vec![skiff_artifact_model::PackageSymbolRef {
                package,
                symbol_path: symbol_path.to_string(),
                abi_expectation: abi.map(str::to_string),
            }],
            ..skiff_artifact_model::ExternalRefTable::default()
        },
        source_map: skiff_artifact_model::SourceMapDto {
            format: String::new(),
            sources: Vec::new(),
            spans: Vec::new(),
        },
        type_table: Vec::new(),
        package_type_records: BTreeMap::from([((owner, symbol_path.to_string()), fields)]),
        link_targets: skiff_artifact_model::FileLinkTargets::default(),
        constants: Vec::new(),
        functions: Vec::new(),
    }
}

#[test]
fn ordinary_compile_without_http_does_not_touch_gateway_only_closure() {
    let unreferenced = projected_fixture("example.com/unreferenced-canonical-candidate").artifact;
    let output = compile_ordinary_source_with_unreferenced_canonical_dependency(
        "example.com/ordinary-zero-gateway-closure",
        "function value() -> bool { return true }\n",
        &unreferenced,
    )
    .expect("ordinary compilation must ignore gateway-only closure resolution");

    assert!(output.package().artifact.package_requirements.is_empty());
    assert!(output.bytecode().is_enabled());
}

/// Compiles an ordinary real `.skiff` fixture while carrying one canonical
/// candidate that is neither manifest-declared nor source-referenced. This is
/// intentionally not a dependency-resolution test: it proves `http = None`
/// never enters the gateway-only reachable-closure resolver.
fn compile_ordinary_source_with_unreferenced_canonical_dependency(
    package_id: &str,
    text: &str,
    dependency: &skiff_artifact_model::PackageArtifact,
) -> Result<PackageCompileOutput, PackageCompileError> {
    let repository_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        crate::CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let temp = std::env::temp_dir().join(format!(
        "skiff-phase4-sleep-{}-{}",
        std::process::id(),
        package_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>(),
    ));
    std::fs::create_dir_all(&temp).expect("create temporary source root");
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).expect("write temporary source");
    let source_tree = crate::SourceTree {
        root: temp.clone(),
        sources: vec![crate::SourceTreeFile {
            module_path: "main".to_string(),
            file_path: std::path::PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        std::path::PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text.to_string(),
        source_path.display().to_string(),
    )
    .expect("parse Phase 4 source fixture");
    let package = crate::PackageSourceInput::new(
        crate::PublicationManifest::new(
            skiff_compiler_core::id::PublicationId::parse(package_id)
                .expect("valid fixture package id"),
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
            crate::ManifestProvenance {
                owner: crate::ManifestOwner::UserOrBuiltinPackage,
                path: std::path::PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        crate::PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let result = crate::compile_package(
        crate::PackageCompileInput::new(&platform_sources, &package, &aliases, package_id, true)
            .with_canonical_dependencies(std::slice::from_ref(dependency), &[]),
    );
    std::fs::remove_dir_all(temp).expect("remove temporary source root");
    result
}
