use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use serde_json::json;
use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, ContractTypeDescriptor, ContractTypeNameability,
    FileIrRef, FileIrUnit, InstructionSourceSite, LiteralIr, NamedUnionBranchIr, PackageArtifact,
    PackageArtifactRef, PackageBinding, PackageBuildId, PackageCodeSlot,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageRequirement,
    PackageRequirementKey, PackageRuntimeRequirements, PackageSchemaCanonicalDescriptor,
    PackageSchemaIndex, PackageSchemaIndexEntry, PackageSchemaIndexRef, PackageSchemaTypeId,
    PackageSchemaTypeRecord, PackageSchemaTypeRecordRef, RuntimeAssembly, SourcePosition,
    SourceSpanRef, SyntheticInstructionSiteReason, TypeDeclIr as ArtifactTypeDecl,
    TypeDescriptorIr, TypeExport, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, FileAddr, HydratedPackageCode, PackageSymbolKey,
    PublicationResourceTable, ServiceErrorTypeIndex, SharedPackageLinkedImage, TypeAddr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, InstantiatedTypeArgumentIdentity,
        LiteralIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity,
        NamedUnionOwnerIdentity, NominalTypeIdentity, PlatformBuiltinErrorIdentity,
        RequestException, ServiceErrorEnvelope,
    },
};

use super::*;
use crate::assembly_execution::RuntimeAssemblyExecutionProjection;

const PROVIDER: &str = "example/provider";
const CALLER: &str = "example/caller";
const RELAY: &str = "example/relay";
const UNLINKED: &str = "example/unlinked";
const DEPENDENCY: &str = "example/dependency";
const REPRESENTATION: &str = "example/representation";
const UNION: &str = "example/union";
const STD: &str = "skiff.run/std";

#[test]
fn internal_message_is_fixed_and_non_diagnostic() {
    assert_eq!(INTERNAL_SERVICE_ERROR_MESSAGE, "Internal service error");
    for forbidden in ["type", "field", "path", "function", "encoder"] {
        assert!(!INTERNAL_SERVICE_ERROR_MESSAGE.contains(forbidden));
    }
}

#[test]
fn cancellation_export_is_terminal_and_produces_no_service_envelope() {
    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let heap = RequestHeap::default();

    let terminal = CanonicalServiceErrorChannel::export_provider_failure(
        &RuntimeError::Cancelled,
        fixture.export_context(&projection, &heap, PROVIDER, Some(CALLER)),
        || panic!("cancellation must not allocate an error correlation"),
    )
    .expect_err("cancellation must not produce a service envelope");

    assert!(terminal.is_cancellation_terminal());
    assert_eq!(terminal.ordinary_payload(), None);
    assert_eq!(terminal.ordinary_catch_projection(), None);
}

#[test]
fn platform_registry_round_trips_exact_identity_payloads() {
    let cases = [
        (
            PlatformBuiltinErrorIdentity::Timeout,
            json!({
                "reason": "deadlineExceeded",
                "instructionCount": 4,
                "limit": null,
                "elapsedMs": 1.5,
            }),
        ),
        (
            PlatformBuiltinErrorIdentity::DbConflict,
            json!({"target": "std.db", "message": "conflict", "retryable": true}),
        ),
        (
            PlatformBuiltinErrorIdentity::DbConstraint,
            json!({
                "kind": "unique",
                "packageId": "example.com/accounts",
                "collection": "user",
            }),
        ),
        (
            PlatformBuiltinErrorIdentity::Http,
            json!({"message": "upstream", "detail": {"status": 503}}),
        ),
    ];
    for (identity, payload) in cases {
        let encoded = encode_platform_payload(identity, &payload).expect("encode");
        assert_eq!(
            decode_platform_payload(identity, &encoded).expect("decode"),
            payload
        );
    }
}

#[test]
fn database_constraint_platform_payload_rejects_backend_details_and_retry_hints() {
    for payload in [
        json!({
            "kind": "unique",
            "packageId": "example.com/accounts",
            "collection": "user",
            "retryable": false,
        }),
        json!({
            "kind": "unique",
            "packageId": "example.com/accounts",
            "collection": "user",
            "index": "_id_",
        }),
        json!({
            "kind": "future",
            "packageId": "example.com/accounts",
            "collection": "user",
        }),
    ] {
        assert!(
            encode_platform_payload(PlatformBuiltinErrorIdentity::DbConstraint, &payload).is_err()
        );
    }
}

#[test]
fn platform_registry_rejects_identity_fallback_and_noncanonical_bytes() {
    let provider = json!({"target": "svc", "reason": "down"});
    let encoded = encode_platform_payload(
        PlatformBuiltinErrorIdentity::ServiceProviderUnavailable,
        &provider,
    )
    .expect("provider payload");
    assert!(
        decode_platform_payload(PlatformBuiltinErrorIdentity::ServiceProtocol, &encoded).is_err()
    );
    assert!(decode_platform_payload(
        PlatformBuiltinErrorIdentity::ServiceProviderUnavailable,
        br#"{ "reason":"down","target":"svc"}"#,
    )
    .is_err());
    assert!(PlatformBuiltinErrorIdentity::from_symbol("std.resource.ResourceError").is_none());
}

#[test]
fn legacy_cancel_platform_identity_is_absent_from_registry() {
    assert!(PlatformBuiltinErrorIdentity::from_symbol("CancelError").is_none());
    assert!(PlatformBuiltinErrorIdentity::from_symbol("std.cancel.CancelError").is_none());
}

#[test]
fn platform_registry_has_no_shape_code_or_message_inference() {
    let payload = json!({"target": "svc", "message": "std.service.ProtocolError"});
    assert!(encode_platform_payload(PlatformBuiltinErrorIdentity::JsonDecode, &payload).is_ok());
    assert!(encode_platform_payload(
        PlatformBuiltinErrorIdentity::ServiceProviderUnavailable,
        &payload,
    )
    .is_err());
}

#[test]
fn record_linked_unlinked_and_three_hop_forward_preserve_exact_bytes() {
    assert_public_reverse_lookup_uses_only_schema_paths();

    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let provider_addr = fixture.addr(PROVIDER, "api.ProviderFault");
    let identity = local_identity(provider_addr.clone());
    let source = call_site();
    let record_correlation = correlation("record");
    let mut provider_heap = RequestHeap::default();
    let value = object(
        &mut provider_heap,
        [("message", RuntimeValue::String("denied".to_string()))],
    );
    let request = RequestException::local(
        RuntimeValueCarrier::identified(value, identity.clone()),
        source.clone(),
        vec![ExceptionStackFrame::Local {
            site: source.clone(),
        }],
        record_correlation.clone(),
    )
    .expect("local provider exception");
    let actual = RuntimeError::UserException(UserException::new(request));
    let correlation_calls = Cell::new(0);
    let fixed = CanonicalServiceErrorChannel::export_provider_failure(
        &actual,
        fixture.export_context(&projection, &provider_heap, PROVIDER, Some(CALLER)),
        || {
            correlation_calls.set(correlation_calls.get() + 1);
            Ok(correlation("unexpected"))
        },
    )
    .expect("record export");
    assert_eq!(correlation_calls.get(), 0);
    assert!(matches!(
        fixed.envelope(),
        ServiceErrorEnvelope::PublicTypedError {
            package_id,
            stable_schema_key,
            trace_id,
            error_id,
            ..
        } if package_id == PROVIDER
            && stable_schema_key == "api.ProviderFault"
            && trace_id == &record_correlation.trace_id
            && error_id == &record_correlation.error_id
    ));

    let mut caller_heap = RequestHeap::default();
    let imported = fixture
        .import(
            &projection,
            fixed.clone(),
            CALLER,
            &mut caller_heap,
            &source,
        )
        .expect("linked caller import");
    assert_eq!(imported.actual_payload_type(), Some(&identity));
    assert_eq!(
        imported.request().stack(),
        &[
            ExceptionStackFrame::Local {
                site: source.clone(),
            },
            ExceptionStackFrame::RemoteBoundary {
                service_id: "svc.provider".to_string(),
                operation_id: "errors.throw".to_string(),
                error_id: record_correlation.error_id.clone(),
            },
        ]
    );

    let forwarded = CanonicalServiceErrorChannel::export_provider_failure(
        &RuntimeError::UserException(imported.clone()),
        fixture.export_context(&projection, &caller_heap, CALLER, Some(RELAY)),
        || panic!("imported cause must forward before correlation allocation"),
    )
    .expect("second-hop raw forward");
    assert_eq!(forwarded.encoded_bytes(), fixed.encoded_bytes());

    let mut relay_heap = RequestHeap::default();
    let relay_imported = fixture
        .import(&projection, forwarded, RELAY, &mut relay_heap, &source)
        .expect("relay import");
    let third = CanonicalServiceErrorChannel::export_provider_failure(
        &RuntimeError::UserException(relay_imported),
        fixture.export_context(&projection, &relay_heap, RELAY, None),
        || panic!("third hop must not allocate a cause"),
    )
    .expect("third-hop raw forward");
    assert_eq!(third.encoded_bytes(), fixed.encoded_bytes());

    let mut unlinked_heap = RequestHeap::default();
    let opaque = fixture
        .import(
            &projection,
            fixed.clone(),
            UNLINKED,
            &mut unlinked_heap,
            &source,
        )
        .expect("valid unlinked public error remains opaque");
    assert!(opaque.request().local_value().is_none());
    assert!(opaque.actual_payload_type().is_none());
    assert_eq!(
        opaque
            .request()
            .fixed_service_error()
            .expect("raw fixed error")
            .encoded_bytes(),
        fixed.encoded_bytes()
    );
}

#[test]
fn dependency_representation_and_named_union_use_exact_owner_and_selection() {
    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let source = call_site();

    let dependency_addr = fixture.addr(DEPENDENCY, "api.DependencyFault");
    let mut dependency_heap = RequestHeap::default();
    let dependency_value = object(
        &mut dependency_heap,
        [("code", RuntimeValue::String("dependency".to_string()))],
    );
    let dependency = local_error(
        dependency_value,
        local_identity(dependency_addr.clone()),
        "dependency",
    );
    let fixed_dependency = CanonicalServiceErrorChannel::export_provider_failure(
        &dependency,
        fixture.export_context(&projection, &dependency_heap, PROVIDER, Some(CALLER)),
        || panic!("local dependency cause already has correlation"),
    )
    .expect("dependency-owned export");
    assert!(matches!(
        fixed_dependency.envelope(),
        ServiceErrorEnvelope::PublicTypedError { package_id, .. }
            if package_id == DEPENDENCY
    ));

    let representation_addr = fixture.addr(REPRESENTATION, "api.CodeFault");
    let representation = local_error(
        RuntimeValue::String("E_DENIED".to_string()),
        local_identity(representation_addr.clone()),
        "representation",
    );
    let representation_heap = RequestHeap::default();
    let fixed_representation = CanonicalServiceErrorChannel::export_provider_failure(
        &representation,
        fixture.export_context(&projection, &representation_heap, PROVIDER, Some(CALLER)),
        || panic!("local representation cause already has correlation"),
    )
    .expect("representation Root export");
    let mut caller_heap = RequestHeap::default();
    let imported_representation = fixture
        .import(
            &projection,
            fixed_representation,
            CALLER,
            &mut caller_heap,
            &source,
        )
        .expect("representation import");
    assert_eq!(
        imported_representation.actual_payload_type(),
        Some(&local_identity(representation_addr))
    );

    let union_addr = fixture.addr(UNION, "api.UnionFault");
    let branch_identity = CatchIdentity::NamedUnionBranch {
        union: NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
            addr: union_addr,
            type_arguments: Vec::new(),
        }),
        branch: NamedUnionBranchIdentity::Literal {
            value: LiteralIdentity::String("right".to_string()),
        },
    };
    let union = local_error(
        RuntimeValue::String("right".to_string()),
        branch_identity.clone(),
        "union",
    );
    let union_heap = RequestHeap::default();
    let fixed_union = CanonicalServiceErrorChannel::export_provider_failure(
        &union,
        fixture.export_context(&projection, &union_heap, PROVIDER, Some(CALLER)),
        || panic!("local union cause already has correlation"),
    )
    .expect("exact union branch export");
    let ServiceErrorEnvelope::PublicTypedError {
        encoded_payload, ..
    } = fixed_union.envelope()
    else {
        panic!("union must be public typed");
    };
    assert_eq!(encoded_payload[5], 1);
    let mut union_caller_heap = RequestHeap::default();
    let imported_union = fixture
        .import(
            &projection,
            fixed_union,
            CALLER,
            &mut union_caller_heap,
            &source,
        )
        .expect("union import");
    assert_eq!(imported_union.actual_payload_type(), Some(&branch_identity));
}

#[test]
fn private_generic_encode_failure_and_runtime_fault_create_one_sanitized_internal() {
    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let private_addr = fixture.addr(PROVIDER, "PrivateFault");
    let private_heap = RequestHeap::default();
    let private = local_error(
        RuntimeValue::String("PRIVATE_VALUE".to_string()),
        local_identity(private_addr),
        "private",
    );
    let fixed_private = CanonicalServiceErrorChannel::export_provider_failure(
        &private,
        fixture.export_context(&projection, &private_heap, PROVIDER, Some(CALLER)),
        || panic!("local private cause already has correlation"),
    )
    .expect("private becomes Internal");
    assert_internal(&fixed_private, "trace-private", "trace-private:error");
    let wire = String::from_utf8_lossy(fixed_private.encoded_bytes());
    assert!(!wire.contains("PRIVATE_VALUE"));
    assert!(!wire.contains("PrivateFault"));

    let generic_addr = fixture.addr(PROVIDER, "GenericFault");
    let generic_identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: generic_addr,
            type_arguments: vec![InstantiatedTypeArgumentIdentity::new(
                r#"{"kind":"builtin","name":"string","args":[]}"#,
            )
            .expect("generic argument identity")],
        },
    ));
    let generic = local_error(
        RuntimeValue::String("GENERIC_PRIVATE".to_string()),
        generic_identity,
        "generic",
    );
    let fixed_generic = CanonicalServiceErrorChannel::export_provider_failure(
        &generic,
        fixture.export_context(&projection, &private_heap, PROVIDER, Some(CALLER)),
        || panic!("local generic cause already has correlation"),
    )
    .expect("nonclosed generic becomes Internal");
    assert_internal(&fixed_generic, "trace-generic", "trace-generic:error");

    let provider_addr = fixture.addr(PROVIDER, "api.ProviderFault");
    let forged_arity_identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: provider_addr.clone(),
            type_arguments: vec![InstantiatedTypeArgumentIdentity::new(
                r#"{"kind":"builtin","name":"string","args":[]}"#,
            )
            .expect("forged argument identity")],
        },
    ));
    let forged_arity = local_error(
        RuntimeValue::String("FORGED_ARITY".to_string()),
        forged_arity_identity,
        "forged-arity",
    );
    let error = CanonicalServiceErrorChannel::export_provider_failure(
        &forged_arity,
        fixture.export_context(&projection, &private_heap, PROVIDER, Some(CALLER)),
        || panic!("invalid artifact must not allocate a second cause"),
    )
    .expect_err("arguments applied to a nongeneric declaration are invalid artifact");
    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));

    let mut mismatched_heap = RequestHeap::default();
    let mismatched_value = object(
        &mut mismatched_heap,
        [("message", RuntimeValue::Number(42.0))],
    );
    let mismatched = local_error(mismatched_value, local_identity(provider_addr), "encode");
    let fixed_mismatch = CanonicalServiceErrorChannel::export_provider_failure(
        &mismatched,
        fixture.export_context(&projection, &mismatched_heap, PROVIDER, Some(CALLER)),
        || panic!("local encoding failure retains its cause"),
    )
    .expect("actual-value encoding failure becomes Internal");
    assert_internal(&fixed_mismatch, "trace-encode", "trace-encode:error");

    let calls = Cell::new(0);
    let runtime_fault = RuntimeError::Unsupported(
        "PRIVATE_TYPE field path encoder function diagnostic".to_string(),
    );
    let fixed_fault = CanonicalServiceErrorChannel::export_provider_failure(
        &runtime_fault,
        fixture.export_context(&projection, &private_heap, PROVIDER, Some(CALLER)),
        || {
            calls.set(calls.get() + 1);
            Ok(correlation("runtime"))
        },
    )
    .expect("runtime fault becomes Internal");
    assert_eq!(calls.get(), 1);
    assert_internal(&fixed_fault, "trace-runtime", "trace-runtime:error");
    let wire = String::from_utf8_lossy(fixed_fault.encoded_bytes());
    assert!(!wire.contains("PRIVATE_TYPE"));
    assert!(!wire.contains("encoder"));
}

struct TestTelemetry;

impl skiff_runtime_capability_context::TelemetryCapabilityApi for TestTelemetry {
    fn emit_native(
        &self,
        _target: &str,
        _args: &[serde_json::Value],
    ) -> skiff_runtime_capability_context::CapabilityResult<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

struct FailingRestrictedServiceDiagnosticSink;

impl skiff_runtime_capability_context::RestrictedServiceDiagnosticSink
    for FailingRestrictedServiceDiagnosticSink
{
    fn submit(
        &self,
        _diagnostic: &RestrictedServiceDiagnostic,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(
            skiff_runtime_capability_context::CapabilityError::provider_unavailable(
                "restricted-service-diagnostic",
                "test sink unavailable",
            ),
        )
    }
}

#[test]
fn restricted_service_diagnostic_private_sink_failure_preserves_fixed_bytes_and_safe_fields() {
    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let provider_heap = RequestHeap::default();
    let private = local_error(
        RuntimeValue::String("provider-private-secret".to_string()),
        local_identity(fixture.addr(PROVIDER, "PrivateFault")),
        "restricted",
    );
    let baseline = CanonicalServiceErrorChannel::export_provider_failure(
        &private,
        fixture.export_context(&projection, &provider_heap, PROVIDER, Some(CALLER)),
        || panic!("local private failure retains its original correlation"),
    )
    .expect("baseline private export");
    let source = call_site();
    let fallback_stack = [ExceptionStackFrame::Local {
        site: source.clone(),
    }];
    let telemetry = TelemetryCapabilityContext::new(TestTelemetry)
        .with_restricted_service_diagnostic_sink(RecordingRestrictedServiceDiagnosticSink);
    let generation = 9_334;
    start_restricted_service_diagnostic_probe_for_test(generation);
    let fixed = CanonicalServiceErrorChannel::export_provider_failure_with_diagnostic(
        &private,
        fixture.export_context(&projection, &provider_heap, PROVIDER, Some(CALLER)),
        RestrictedServiceDiagnosticExportContext {
            telemetry: &telemetry,
            provider_activation_id: "activation:provider",
            request_generation: generation,
            fallback_source: &source,
            fallback_stack: &fallback_stack,
        },
        || panic!("local private failure retains its original correlation"),
    )
    .expect("restricted private export");
    assert_eq!(fixed.encoded_bytes(), baseline.encoded_bytes());

    let diagnostics = take_restricted_service_diagnostics_for_test(generation);
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.correlation.trace_id, fixed.envelope().trace_id());
    assert_eq!(diagnostic.correlation.error_id, fixed.envelope().error_id());
    assert_eq!(
        diagnostic.cause_kind,
        RestrictedServiceDiagnosticCauseKind::InternalError
    );
    assert_eq!(diagnostic.source, source);
    assert_eq!(diagnostic.stack, fallback_stack);
    assert!(!format!("{diagnostic:?}").contains("provider-private-secret"));

    let failing = TelemetryCapabilityContext::new(TestTelemetry)
        .with_restricted_service_diagnostic_sink(FailingRestrictedServiceDiagnosticSink);
    let fixed_with_failed_sink =
        CanonicalServiceErrorChannel::export_provider_failure_with_diagnostic(
            &private,
            fixture.export_context(&projection, &provider_heap, PROVIDER, Some(CALLER)),
            RestrictedServiceDiagnosticExportContext {
                telemetry: &failing,
                provider_activation_id: "activation:provider",
                request_generation: generation + 1,
                fallback_source: &source,
                fallback_stack: &fallback_stack,
            },
            || panic!("local private failure retains its original correlation"),
        )
        .expect("sink failure cannot replace the service error");
    assert_eq!(
        fixed_with_failed_sink.encoded_bytes(),
        baseline.encoded_bytes()
    );
}

#[test]
fn restricted_service_diagnostic_platform_uses_typed_source_and_final_correlation() {
    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let provider_heap = RequestHeap::default();
    let span = SourceSpanRef {
        source_id: 7,
        start: SourcePosition::new(11, 3),
        end: SourcePosition::new(11, 12),
    };
    let actual = RuntimeError::FileError {
        message: "provider-private-file-message".to_string(),
    }
    .with_source(
        42,
        json!({
            "span": {
                "id": 42,
                "source": 7,
                "kind": "CallExpression",
                "span": span.clone(),
            },
            "source": {
                "id": 7,
                "path": "/provider/private/source.skiff",
                "modulePath": "provider.private",
            },
        }),
    );
    let fallback_source = call_site();
    let fallback_stack = [ExceptionStackFrame::Local {
        site: fallback_source.clone(),
    }];
    let telemetry = TelemetryCapabilityContext::new(TestTelemetry)
        .with_restricted_service_diagnostic_sink(RecordingRestrictedServiceDiagnosticSink);
    let generation = 9_335;
    start_restricted_service_diagnostic_probe_for_test(generation);
    let fixed = CanonicalServiceErrorChannel::export_provider_failure_with_diagnostic(
        &actual,
        fixture.export_context(&projection, &provider_heap, PROVIDER, Some(CALLER)),
        RestrictedServiceDiagnosticExportContext {
            telemetry: &telemetry,
            provider_activation_id: "activation:provider",
            request_generation: generation,
            fallback_source: &fallback_source,
            fallback_stack: &fallback_stack,
        },
        || Ok(correlation("platform-restricted")),
    )
    .expect("platform failure should be fixed and diagnosed");

    let diagnostics = take_restricted_service_diagnostics_for_test(generation);
    assert_eq!(diagnostics.len(), 1);
    let expected_source = InstructionSourceSite::Source { span };
    assert_eq!(diagnostics[0].source, expected_source);
    assert_eq!(
        diagnostics[0].stack,
        vec![ExceptionStackFrame::Local {
            site: expected_source,
        }]
    );
    assert_eq!(
        diagnostics[0].cause_kind,
        RestrictedServiceDiagnosticCauseKind::PlatformError
    );
    assert_eq!(
        diagnostics[0].correlation.trace_id,
        fixed.envelope().trace_id()
    );
    assert_eq!(
        diagnostics[0].correlation.error_id,
        fixed.envelope().error_id()
    );
    let safe = format!("{:?}", diagnostics[0]);
    assert!(!safe.contains("provider-private-file-message"));
    assert!(!safe.contains("/provider/private/source.skiff"));
    assert!(!fixed
        .encoded_bytes()
        .windows(b"/provider/private/source.skiff".len())
        .any(|window| window == b"/provider/private/source.skiff"));
}

#[test]
fn exact_local_and_imported_internal_materialize_once_and_forward_raw() {
    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let internal_addr = fixture.addr(STD, INTERNAL_ERROR_STABLE_SCHEMA_KEY);
    let mut provider_heap = RequestHeap::default();
    let value = object(
        &mut provider_heap,
        [
            ("message", RuntimeValue::String("chosen".to_string())),
            ("traceId", RuntimeValue::String("payload-trace".to_string())),
            ("errorId", RuntimeValue::String("payload-error".to_string())),
        ],
    );
    let local = local_error(
        value,
        local_identity(internal_addr.clone()),
        "local-internal",
    );
    let fixed = CanonicalServiceErrorChannel::export_provider_failure(
        &local,
        fixture.export_context(&projection, &provider_heap, PROVIDER, Some(CALLER)),
        || panic!("local InternalError already has correlation"),
    )
    .expect("exact local InternalError uses fixed Internal branch");
    assert_internal(&fixed, "trace-local-internal", "trace-local-internal:error");

    let source = call_site();
    let mut caller_heap = RequestHeap::default();
    let imported = fixture
        .import(
            &projection,
            fixed.clone(),
            CALLER,
            &mut caller_heap,
            &source,
        )
        .expect("fixed Internal materializes caller-local std record");
    assert_eq!(
        imported.actual_payload_type(),
        Some(&local_identity(internal_addr))
    );
    assert_eq!(
        imported
            .request()
            .fixed_service_error()
            .expect("imported raw Internal")
            .encoded_bytes(),
        fixed.encoded_bytes()
    );
    let forwarded = CanonicalServiceErrorChannel::export_provider_failure(
        &RuntimeError::UserException(imported),
        fixture.export_context(&projection, &caller_heap, CALLER, Some(RELAY)),
        || panic!("imported Internal must not allocate a second cause"),
    )
    .expect("imported Internal raw forward");
    assert_eq!(forwarded.encoded_bytes(), fixed.encoded_bytes());

    let fixed_carrier = RuntimeError::FixedServiceFailure(fixed.clone())
        .with_source(91, serde_json::json!({"private": "diagnostic"}));
    let forwarded_carrier = CanonicalServiceErrorChannel::export_provider_failure(
        &fixed_carrier,
        fixture.export_context(&projection, &caller_heap, CALLER, Some(RELAY)),
        || panic!("fixed RuntimeError carrier must not allocate a second cause"),
    )
    .expect("fixed RuntimeError carrier raw forward");
    assert_eq!(forwarded_carrier.encoded_bytes(), fixed.encoded_bytes());
}

#[test]
fn platform_round_trip_is_exact_and_resource_is_never_platform() {
    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let heap = RequestHeap::default();
    let fixed = CanonicalServiceErrorChannel::export_provider_failure(
        &RuntimeError::FileError {
            message: "denied".to_string(),
        },
        fixture.export_context(&projection, &heap, PROVIDER, Some(CALLER)),
        || Ok(correlation("platform")),
    )
    .expect("FileError platform export");
    assert!(matches!(
        fixed.envelope(),
        ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: PlatformBuiltinErrorIdentity::File,
            ..
        }
    ));
    let mut caller_heap = RequestHeap::default();
    let imported = fixture
        .import(&projection, fixed, CALLER, &mut caller_heap, &call_site())
        .expect("platform import");
    assert_eq!(
        imported.actual_payload_type(),
        Some(&PlatformBuiltinErrorIdentity::File.catch_identity())
    );

    let resource_runtime = CanonicalServiceErrorChannel::export_provider_failure(
        &RuntimeError::ResourceError {
            path: "private/path".to_string(),
            message: "private".to_string(),
        },
        fixture.export_context(&projection, &heap, PROVIDER, Some(CALLER)),
        || Ok(correlation("resource-runtime")),
    )
    .expect("generic resource runtime error sanitizes");
    assert!(matches!(
        resource_runtime.envelope(),
        ServiceErrorEnvelope::InternalError { .. }
    ));

    let resource_addr = fixture.addr(STD, "std.resource.ResourceError");
    let mut resource_heap = RequestHeap::default();
    let resource_value = object(
        &mut resource_heap,
        [(
            "message",
            RuntimeValue::String("public resource".to_string()),
        )],
    );
    let resource_local = local_error(
        resource_value,
        local_identity(resource_addr),
        "resource-local",
    );
    let fixed_resource = CanonicalServiceErrorChannel::export_provider_failure(
        &resource_local,
        fixture.export_context(&projection, &resource_heap, PROVIDER, Some(CALLER)),
        || panic!("local public resource cause already has correlation"),
    )
    .expect("ResourceError is ordinary Package public error");
    assert!(matches!(
        fixed_resource.envelope(),
        ServiceErrorEnvelope::PublicTypedError {
            package_id,
            stable_schema_key,
            ..
        } if package_id == STD && stable_schema_key == "std.resource.ResourceError"
    ));
}

#[test]
fn identity_ordinal_and_payload_mutations_fail_closed_while_unknown_owner_stays_opaque() {
    assert_public_reverse_lookup_artifact_failures();

    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let source = call_site();
    let provider_addr = fixture.addr(PROVIDER, "api.ProviderFault");
    let mut heap = RequestHeap::default();
    let value = object(
        &mut heap,
        [("message", RuntimeValue::String("denied".to_string()))],
    );
    let fixed = CanonicalServiceErrorChannel::export_provider_failure(
        &local_error(value, local_identity(provider_addr), "mutation"),
        fixture.export_context(&projection, &heap, PROVIDER, Some(CALLER)),
        || panic!("local cause already has correlation"),
    )
    .expect("public fixture");
    let ServiceErrorEnvelope::PublicTypedError {
        package_id,
        package_schema_type_id,
        encoded_payload,
        trace_id,
        error_id,
        ..
    } = fixed.envelope()
    else {
        panic!("public fixture envelope");
    };

    let wrong_key = fixed_error(ServiceErrorEnvelope::PublicTypedError {
        package_id: package_id.clone(),
        stable_schema_key: "api.Forged".to_string(),
        package_schema_type_id: package_schema_type_id.clone(),
        encoded_payload: encoded_payload.clone(),
        trace_id: trace_id.clone(),
        error_id: error_id.clone(),
    })
    .expect("strict forged identity envelope");
    let mut caller_heap = RequestHeap::default();
    let error = fixture
        .import(&projection, wrong_key, CALLER, &mut caller_heap, &source)
        .expect_err("partial known-owner identity conflict must fail");
    assert!(matches!(error, RuntimeError::Protocol { .. }));

    let wrong_owner = fixed_error(ServiceErrorEnvelope::PublicTypedError {
        package_id: CALLER.to_string(),
        stable_schema_key: "api.ProviderFault".to_string(),
        package_schema_type_id: package_schema_type_id.clone(),
        encoded_payload: encoded_payload.clone(),
        trace_id: trace_id.clone(),
        error_id: error_id.clone(),
    })
    .expect("strict owner-mutation envelope");
    let mut caller_heap = RequestHeap::default();
    let error = fixture
        .import(&projection, wrong_owner, CALLER, &mut caller_heap, &source)
        .expect_err("known Package owner mutation must fail");
    assert!(matches!(error, RuntimeError::Protocol { .. }));

    let wrong_type_id = fixed_error(ServiceErrorEnvelope::PublicTypedError {
        package_id: package_id.clone(),
        stable_schema_key: "api.ProviderFault".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("forged:type-id"),
        encoded_payload: encoded_payload.clone(),
        trace_id: trace_id.clone(),
        error_id: error_id.clone(),
    })
    .expect("strict type-id-mutation envelope");
    let mut caller_heap = RequestHeap::default();
    let error = fixture
        .import(
            &projection,
            wrong_type_id,
            CALLER,
            &mut caller_heap,
            &source,
        )
        .expect_err("known Package type-id mutation must fail");
    assert!(matches!(error, RuntimeError::Protocol { .. }));

    let forged_build = PackageBuildId::new("build:forged-caller");
    let caller_addr = fixture.executable_addr(CALLER);
    let stack = [ExceptionStackFrame::Local {
        site: source.clone(),
    }];
    let mut caller_heap = RequestHeap::default();
    let error = CanonicalServiceErrorChannel::import_caller_failure(
        fixed.clone(),
        ServiceErrorImportContext {
            execution_image: &fixture.image,
            type_view: projection.type_view(),
            caller_heap: &mut caller_heap,
            caller_package_build_id: &forged_build,
            caller_executable_addr: &caller_addr,
            call_site: &source,
            caller_stack_at_site: &stack,
            remote_service_id: "svc.provider",
            remote_operation_id: "errors.throw",
        },
    )
    .expect_err("unknown caller build must fail closed");
    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));

    let mut payload = encoded_payload.clone();
    payload.push(0);
    let wrong_payload = fixed_error(ServiceErrorEnvelope::PublicTypedError {
        package_id: package_id.clone(),
        stable_schema_key: "api.ProviderFault".to_string(),
        package_schema_type_id: package_schema_type_id.clone(),
        encoded_payload: payload,
        trace_id: trace_id.clone(),
        error_id: error_id.clone(),
    })
    .expect("strict payload-mutation envelope");
    let mut caller_heap = RequestHeap::default();
    assert!(matches!(
        fixture
            .import(
                &projection,
                wrong_payload,
                CALLER,
                &mut caller_heap,
                &source,
            )
            .expect_err("payload mutation must fail"),
        RuntimeError::Protocol { .. }
    ));

    let unknown = fixed_error(ServiceErrorEnvelope::PublicTypedError {
        package_id: "outside/assembly".to_string(),
        stable_schema_key: "api.ExternalFault".to_string(),
        package_schema_type_id: PackageSchemaTypeId::new("external:type"),
        encoded_payload: vec![1],
        trace_id: "trace-external".to_string(),
        error_id: "error-external".to_string(),
    })
    .expect("syntactically valid unknown envelope");
    let mut caller_heap = RequestHeap::default();
    let imported = fixture
        .import(&projection, unknown, CALLER, &mut caller_heap, &source)
        .expect("fully unknown owner stays opaque");
    assert!(imported.request().local_value().is_none());

    let union_addr = fixture.addr(UNION, "api.UnionFault");
    let union = local_error(
        RuntimeValue::String("right".to_string()),
        CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
                addr: union_addr,
                type_arguments: Vec::new(),
            }),
            branch: NamedUnionBranchIdentity::Literal {
                value: LiteralIdentity::String("right".to_string()),
            },
        },
        "ordinal",
    );
    let union_heap = RequestHeap::default();
    let fixed_union = CanonicalServiceErrorChannel::export_provider_failure(
        &union,
        fixture.export_context(&projection, &union_heap, PROVIDER, Some(CALLER)),
        || panic!("local union cause already has correlation"),
    )
    .expect("union fixture");
    let ServiceErrorEnvelope::PublicTypedError {
        package_id,
        stable_schema_key,
        package_schema_type_id,
        encoded_payload,
        trace_id,
        error_id,
    } = fixed_union.envelope()
    else {
        panic!("union public fixture");
    };
    let mut encoded_payload = encoded_payload.clone();
    encoded_payload[5] = 9;
    let wrong_ordinal = fixed_error(ServiceErrorEnvelope::PublicTypedError {
        package_id: package_id.clone(),
        stable_schema_key: stable_schema_key.clone(),
        package_schema_type_id: package_schema_type_id.clone(),
        encoded_payload,
        trace_id: trace_id.clone(),
        error_id: error_id.clone(),
    })
    .expect("strict ordinal-mutation envelope");
    let mut caller_heap = RequestHeap::default();
    assert!(matches!(
        fixture
            .import(
                &projection,
                wrong_ordinal,
                CALLER,
                &mut caller_heap,
                &source,
            )
            .expect_err("ordinal mutation must fail"),
        RuntimeError::Protocol { .. }
    ));
}

#[test]
fn malformed_envelope_and_platform_payload_never_fall_back_to_internal() {
    assert!(OpaqueServiceError::decode(
        br#"{"kind":"internalError","payload":{"message":"x","traceId":"","errorId":"e"}}"#
            .to_vec(),
    )
    .is_err());

    let fixture = CoreFixture::new();
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&fixture.image));
    let malformed_platform = fixed_error(ServiceErrorEnvelope::PlatformError {
        builtin_error_identity: PlatformBuiltinErrorIdentity::File,
        encoded_payload: canonical_json_bytes(&json!({"target": "not-a-file"}))
            .expect("canonical malformed platform bytes"),
        trace_id: "trace-platform-invalid".to_string(),
        error_id: "error-platform-invalid".to_string(),
    })
    .expect("outer envelope is strict");
    let mut heap = RequestHeap::default();
    let error = fixture
        .import(
            &projection,
            malformed_platform,
            CALLER,
            &mut heap,
            &call_site(),
        )
        .expect_err("identity-selected platform validator must reject payload");
    assert!(matches!(error, RuntimeError::Protocol { .. }));

    let noncanonical_internal = fixed_error(ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: "provider diagnostic".to_string(),
            trace_id: "trace-internal-invalid".to_string(),
            error_id: "error-internal-invalid".to_string(),
        },
    })
    .expect("outer Internal envelope is strict");
    let mut heap = RequestHeap::default();
    let error = fixture
        .import(
            &projection,
            noncanonical_internal,
            CALLER,
            &mut heap,
            &call_site(),
        )
        .expect_err("Internal message must be the one canonical safe constant");
    assert!(matches!(error, RuntimeError::Protocol { .. }));
}

#[test]
fn local_rethrow_reuses_the_same_cause_source_stack_and_correlation() {
    let site = call_site();
    let expected = RequestException::local(
        RuntimeValueCarrier::identified(
            RuntimeValue::String("local".to_string()),
            PlatformBuiltinErrorIdentity::File.catch_identity(),
        ),
        site.clone(),
        vec![ExceptionStackFrame::Local { site }],
        correlation("rethrow"),
    )
    .expect("local exception");
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_exception(expected.clone())
        .expect("exception heap node");
    let carrier = RuntimeValueCarrier::unidentified(RuntimeValue::Heap(handle));

    let rethrown =
        crate::exceptions::request_exception_for_rethrow(&carrier, &heap).expect("local rethrow");

    assert_eq!(rethrown, expected);
    assert!(rethrown.fixed_service_error().is_none());
}

#[test]
fn provider_stack_scope_resets_local_frames_but_keeps_request_sequence_shared() {
    let sequence = Arc::new(AtomicU64::new(11));
    let provider_sequence = Arc::clone(&sequence);
    let mut inherited = vec![ExceptionStackFrame::Local { site: call_site() }];

    crate::program_execution::reset_provider_local_stack(&mut inherited);

    assert!(inherited.is_empty());
    assert!(Arc::ptr_eq(&sequence, &provider_sequence));
    assert_eq!(provider_sequence.fetch_add(1, Ordering::Relaxed), 11);
}

fn local_error(value: RuntimeValue, identity: CatchIdentity, label: &str) -> RuntimeError {
    let site = call_site();
    let request = RequestException::local(
        RuntimeValueCarrier::identified(value, identity),
        site.clone(),
        vec![ExceptionStackFrame::Local { site }],
        correlation(label),
    )
    .expect("local exception fixture");
    RuntimeError::UserException(UserException::new(request))
}

fn local_identity(addr: TypeAddr) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr,
            type_arguments: Vec::new(),
        },
    ))
}

fn correlation(label: &str) -> ErrorCorrelation {
    ErrorCorrelation {
        trace_id: format!("trace-{label}"),
        error_id: format!("trace-{label}:error"),
    }
}

fn call_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn object(
    heap: &mut RequestHeap,
    fields: impl IntoIterator<Item = (&'static str, RuntimeValue)>,
) -> RuntimeValue {
    let fields = fields
        .into_iter()
        .map(|(name, value)| (name.to_string(), RuntimeValueCarrier::unidentified(value)))
        .collect();
    RuntimeValue::Heap(
        heap.alloc_object_carriers(fields)
            .expect("object fixture allocation"),
    )
}

fn assert_internal(error: &OpaqueServiceError, trace_id: &str, error_id: &str) {
    assert_eq!(
        error.envelope(),
        &ServiceErrorEnvelope::InternalError {
            payload: InternalErrorPayload {
                message: INTERNAL_SERVICE_ERROR_MESSAGE.to_string(),
                trace_id: trace_id.to_string(),
                error_id: error_id.to_string(),
            },
        }
    );
}

fn assert_public_reverse_lookup_uses_only_schema_paths() {
    let mut public = reverse_lookup_package();
    let public_export = public.artifact.implementation_links.types["Failure"].clone();
    Arc::make_mut(&mut public.artifact)
        .implementation_links
        .types
        .insert("main.Failure".to_string(), public_export);
    let image = link_single_package_fixture(&public);
    let identity = public_artifact_identity_for_addr(&image, &single_package_type_addr(0))
        .expect("public reverse lookup")
        .expect("public Failure identity");
    let expected = &public.index.types["Failure"];
    assert_eq!(identity.package_id(), public.package_id);
    assert_eq!(identity.stable_schema_key(), "Failure");
    assert_eq!(
        identity.package_schema_type_id(),
        &expected.package_schema_type_id
    );

    let mut implementation_only = reverse_lookup_package();
    let source_export = implementation_only.artifact.implementation_links.types["Failure"].clone();
    Arc::make_mut(&mut implementation_only.artifact)
        .implementation_links
        .types
        .insert("main.Failure".to_string(), source_export);
    Arc::make_mut(&mut implementation_only.index).types.clear();
    implementation_only.records.clear();
    Arc::make_mut(&mut implementation_only.artifact)
        .package_schema_type_records
        .clear();
    refresh_schema_index_identity(&mut implementation_only);
    let image = link_single_package_fixture(&implementation_only);
    assert_eq!(
        public_artifact_identity_for_addr(&image, &single_package_type_addr(0))
            .expect("implementation-only reverse lookup"),
        None
    );
}

fn assert_public_reverse_lookup_artifact_failures() {
    let mut ambiguous = reverse_lookup_package();
    let baseline = link_single_package_fixture(&ambiguous);
    add_second_public_identity_at_failure_addr(&mut ambiguous);
    let (assembly, hydrated) = single_package_inputs(&ambiguous);
    let admission =
        skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, [hydrated])
            .expect_err("assembly admission must reject two public identities at one address");
    assert!(
        format!("{admission:#}").contains("multiple public Package schema identities"),
        "{admission:#}"
    );
    let image = unchecked_reverse_lookup_image(&baseline, &ambiguous);
    assert_invalid_reverse_lookup(&image, "multiple public Package schema identities");

    let mut missing_path = reverse_lookup_package();
    let baseline = link_single_package_fixture(&missing_path);
    Arc::make_mut(&mut missing_path.index)
        .types
        .get_mut("Failure")
        .expect("Failure schema entry")
        .public_path = None;
    refresh_schema_index_identity(&mut missing_path);
    let image = unchecked_reverse_lookup_image(&baseline, &missing_path);
    assert_invalid_reverse_lookup(&image, "Package schema index is invalid");

    let mut forged_path = reverse_lookup_package();
    let baseline = link_single_package_fixture(&forged_path);
    let source_export = forged_path.artifact.implementation_links.types["Failure"].clone();
    Arc::make_mut(&mut forged_path.artifact)
        .implementation_links
        .types
        .insert("main.Failure".to_string(), source_export);
    Arc::make_mut(&mut forged_path.index)
        .types
        .get_mut("Failure")
        .expect("Failure schema entry")
        .public_path = Some("main.Failure".to_string());
    refresh_schema_index_identity(&mut forged_path);
    let image = unchecked_reverse_lookup_image(&baseline, &forged_path);
    assert_invalid_reverse_lookup(&image, "Package schema index is invalid");

    let mut missing_link = reverse_lookup_package();
    let baseline = link_single_package_fixture(&missing_link);
    Arc::make_mut(&mut missing_link.artifact)
        .implementation_links
        .types
        .remove("Failure");
    let image = unchecked_reverse_lookup_image(&baseline, &missing_link);
    assert_invalid_reverse_lookup(&image, "no exact implementation type link");

    let mut missing_coordinate = reverse_lookup_package();
    let baseline = link_single_package_fixture(&missing_coordinate);
    Arc::make_mut(&mut missing_coordinate.artifact)
        .implementation_links
        .types
        .get_mut("Failure")
        .expect("Failure public link")
        .type_index = 99;
    let image = unchecked_reverse_lookup_image(&baseline, &missing_coordinate);
    assert_invalid_reverse_lookup(&image, "missing type declaration");

    let mut forged_record = reverse_lookup_package();
    let baseline = link_single_package_fixture(&forged_record);
    let type_id = forged_record.index.types["Failure"]
        .package_schema_type_id
        .clone();
    Arc::make_mut(
        forged_record
            .records
            .get_mut(&type_id)
            .expect("Failure loaded record"),
    )
    .stable_schema_key = "ForgedFailure".to_string();
    let image = unchecked_reverse_lookup_image(&baseline, &forged_record);
    assert_invalid_reverse_lookup(&image, "owner/key/type-id invariant is broken");
}

fn assert_invalid_reverse_lookup(image: &AssemblyExecutionImage, expected: &str) {
    let error = public_artifact_identity_for_addr(image, &single_package_type_addr(0))
        .expect_err("malformed public reverse lookup must fail closed");
    let RuntimeError::InvalidArtifact(message) = error else {
        panic!("reverse lookup returned the wrong error kind: {error:?}");
    };
    assert!(message.contains(expected), "{message}");
}

fn reverse_lookup_package() -> PackageFixture {
    package(
        "example/reverse-lookup",
        vec![PublicType::public(
            "Failure",
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([("message".to_string(), TypeRefIr::builtin("string"))]),
            },
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "message".to_string(),
                    ContractTypeRef::builtin("string"),
                )]),
            },
        )],
    )
}

fn add_second_public_identity_at_failure_addr(package: &mut PackageFixture) {
    let first_entry = package.index.types["Failure"].clone();
    let first_record = package.records[&first_entry.package_schema_type_id].clone();
    let stable_schema_key = "AlternateFailure";
    let type_id = skiff_artifact_identity::package_schema_type_id(
        &package.package_id,
        stable_schema_key,
        &first_record.canonical_descriptor,
    )
    .expect("alternate public identity");
    Arc::make_mut(&mut package.index).types.insert(
        stable_schema_key.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: type_id.clone(),
            public_path: Some(stable_schema_key.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    );
    package.records.insert(
        type_id.clone(),
        Arc::new(PackageSchemaTypeRecord {
            package_id: package.package_id.clone(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: type_id.clone(),
            canonical_descriptor: first_record.canonical_descriptor.clone(),
        }),
    );
    let failure_export = package.artifact.implementation_links.types["Failure"].clone();
    let artifact = Arc::make_mut(&mut package.artifact);
    artifact.package_schema_type_records.insert(
        type_id.clone(),
        PackageSchemaTypeRecordRef {
            package_id: package.package_id.clone(),
            package_schema_type_id: type_id,
        },
    );
    artifact
        .implementation_links
        .types
        .insert(stable_schema_key.to_string(), failure_export);
    refresh_schema_index_identity(package);
}

fn refresh_schema_index_identity(package: &mut PackageFixture) {
    let index = Arc::make_mut(&mut package.index);
    index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(&index.package_id, &index.types)
            .expect("refreshed schema index identity");
    Arc::make_mut(&mut package.artifact)
        .package_schema_index
        .package_schema_index_identity = index.package_schema_index_identity.clone();
}

fn single_package_type_addr(type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

fn single_package_inputs(package: &PackageFixture) -> (RuntimeAssembly, HydratedPackageCode) {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:service-error-reverse-lookup"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package.artifact_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package.artifact_ref.clone(),
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let hydrated = HydratedPackageCode::new(
        Arc::clone(&package.artifact),
        vec![Arc::clone(&package.file)],
        PublicationResourceTable::default(),
    )
    .with_schema_index(Arc::clone(&package.index))
    .with_schema_records(package.records.clone());
    (assembly, hydrated)
}

fn link_single_package_fixture(package: &PackageFixture) -> Arc<AssemblyExecutionImage> {
    let (assembly, hydrated) = single_package_inputs(package);
    skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, [hydrated])
        .expect("link single-Package reverse-lookup fixture")
}

fn unchecked_reverse_lookup_image(
    baseline: &Arc<AssemblyExecutionImage>,
    package: &PackageFixture,
) -> Arc<AssemblyExecutionImage> {
    let (assembly, hydrated) = single_package_inputs(package);
    let shared = Arc::new(
        SharedPackageLinkedImage::from_runtime_assembly(&assembly, [hydrated])
            .expect("hydrate unchecked reverse-lookup fixture"),
    );
    let mut types = baseline.types().clone();
    for (public_path, export) in &package.artifact.implementation_links.types {
        let file_index = package
            .artifact
            .files
            .iter()
            .position(|file| file.file_ir_identity == export.file.file_ir_identity)
            .expect("fixture implementation link targets its loaded File IR");
        types.exported_types.insert_package(
            PackageSymbolKey::new(0, public_path.clone()),
            TypeAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::LoadedFileIndex(file_index),
                type_index: export.type_index as usize,
            },
        );
    }
    Arc::new(
        AssemblyExecutionImage::try_new(
            shared,
            baseline.execution_packages().to_vec(),
            types,
            Arc::new(ServiceErrorTypeIndex::default()),
        )
        .expect("construct function-level reverse-lookup image"),
    )
}

struct CoreFixture {
    image: Arc<AssemblyExecutionImage>,
    builds: BTreeMap<String, PackageBuildId>,
    slots: BTreeMap<String, usize>,
    type_indices: BTreeMap<(String, String), usize>,
}

impl CoreFixture {
    fn new() -> Self {
        let record = |field: &str| {
            (
                TypeDescriptorIr::Record {
                    fields: BTreeMap::from([(field.to_string(), TypeRefIr::builtin("string"))]),
                },
                ContractTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        field.to_string(),
                        ContractTypeRef::builtin("string"),
                    )]),
                },
            )
        };
        let (provider_source, provider_schema) = record("message");
        let (dependency_source, dependency_schema) = record("code");
        let (marker_source, marker_schema) = record("marker");
        let (internal_source, internal_schema) = (
            TypeDescriptorIr::Record {
                fields: BTreeMap::from([
                    ("errorId".to_string(), TypeRefIr::builtin("string")),
                    ("message".to_string(), TypeRefIr::builtin("string")),
                    ("traceId".to_string(), TypeRefIr::builtin("string")),
                ]),
            },
            ContractTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("errorId".to_string(), ContractTypeRef::builtin("string")),
                    ("message".to_string(), ContractTypeRef::builtin("string")),
                    ("traceId".to_string(), ContractTypeRef::builtin("string")),
                ]),
            },
        );
        let (resource_source, resource_schema) = record("message");
        let mut packages = vec![
            package(
                PROVIDER,
                vec![
                    PublicType::public("api.ProviderFault", provider_source, provider_schema),
                    PublicType::private(
                        "PrivateFault",
                        TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "secret".to_string(),
                                TypeRefIr::builtin("string"),
                            )]),
                        },
                        Vec::new(),
                    ),
                    PublicType::private(
                        "GenericFault",
                        TypeDescriptorIr::Record {
                            fields: BTreeMap::from([(
                                "value".to_string(),
                                TypeRefIr::TypeParam {
                                    name: "T".to_string(),
                                },
                            )]),
                        },
                        vec!["T".to_string()],
                    ),
                ],
            ),
            package(
                CALLER,
                vec![PublicType::public(
                    "api.CallerMarker",
                    marker_source.clone(),
                    marker_schema.clone(),
                )],
            ),
            package(
                RELAY,
                vec![PublicType::public(
                    "api.RelayMarker",
                    marker_source.clone(),
                    marker_schema.clone(),
                )],
            ),
            package(
                UNLINKED,
                vec![PublicType::public(
                    "api.UnlinkedMarker",
                    marker_source,
                    marker_schema,
                )],
            ),
            package(
                DEPENDENCY,
                vec![PublicType::public(
                    "api.DependencyFault",
                    dependency_source,
                    dependency_schema,
                )],
            ),
            package(
                REPRESENTATION,
                vec![PublicType::public(
                    "api.CodeFault",
                    TypeDescriptorIr::Representation {
                        representation: TypeRefIr::builtin("string"),
                    },
                    ContractTypeDescriptor::Representation {
                        target: ContractTypeRef::builtin("string"),
                    },
                )],
            ),
            package(
                UNION,
                vec![PublicType::public(
                    "api.UnionFault",
                    TypeDescriptorIr::Union {
                        branches: vec![
                            NamedUnionBranchIr::Literal {
                                value: LiteralIr::String {
                                    value: "left".to_string(),
                                },
                            },
                            NamedUnionBranchIr::Literal {
                                value: LiteralIr::String {
                                    value: "right".to_string(),
                                },
                            },
                        ],
                    },
                    ContractTypeDescriptor::Enumeration {
                        variants: vec!["left".to_string(), "right".to_string()],
                    },
                )],
            ),
            package(
                STD,
                vec![
                    PublicType::public(
                        INTERNAL_ERROR_STABLE_SCHEMA_KEY,
                        internal_source,
                        internal_schema,
                    ),
                    PublicType::public(
                        "std.resource.ResourceError",
                        resource_source,
                        resource_schema,
                    ),
                ],
            ),
        ];

        let refs = packages
            .iter()
            .map(|package| (package.package_id.clone(), package.artifact_ref.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut bindings = Vec::new();
        for (caller, dependencies) in [
            (PROVIDER, vec![DEPENDENCY, REPRESENTATION, UNION, STD]),
            (
                CALLER,
                vec![PROVIDER, DEPENDENCY, REPRESENTATION, UNION, STD],
            ),
            (RELAY, vec![PROVIDER, STD]),
            (UNLINKED, vec![STD]),
        ] {
            let package = packages
                .iter_mut()
                .find(|package| package.package_id == caller)
                .expect("caller package fixture");
            for dependency in dependencies {
                let dependency_ref = refs.get(dependency).expect("dependency ref");
                let alias = format!("dep{}", bindings.len());
                Arc::make_mut(&mut package.artifact)
                    .package_requirements
                    .push(PackageRequirement {
                        alias: alias.clone(),
                        package_id: dependency_ref.package_id.clone(),
                        exact_version: dependency_ref.package_version.clone(),
                        expected_local_abi: dependency_ref.package_local_abi_identity.clone(),
                        expected_package_build: Some(dependency_ref.package_build_id.clone()),
                    });
                bindings.push(PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: package.artifact_ref.package_build_id.clone(),
                        package_requirement_alias: alias,
                    },
                    package: dependency_ref.clone(),
                });
            }
        }

        let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new("assembly:service-error-core"),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: packages
                .iter()
                .map(|package| package.artifact_ref.clone())
                .collect(),
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: packages
                    .iter()
                    .map(|package| PackageCodeSlot {
                        package: package.artifact_ref.clone(),
                    })
                    .collect(),
                package_links: bindings,
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let hydrated = packages
            .iter()
            .map(|package| {
                HydratedPackageCode::new(
                    Arc::clone(&package.artifact),
                    vec![Arc::clone(&package.file)],
                    PublicationResourceTable::default(),
                )
                .with_schema_index(Arc::clone(&package.index))
                .with_schema_records(package.records.clone())
            })
            .collect::<Vec<_>>();
        let image =
            skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, hydrated)
                .expect("link canonical service-error fixture");
        let builds = packages
            .iter()
            .map(|package| {
                (
                    package.package_id.clone(),
                    package.artifact_ref.package_build_id.clone(),
                )
            })
            .collect();
        let slots = packages
            .iter()
            .enumerate()
            .map(|(slot, package)| (package.package_id.clone(), slot))
            .collect();
        let type_indices =
            packages
                .iter()
                .flat_map(|package| {
                    package.types.iter().enumerate().map(move |(index, ty)| {
                        ((package.package_id.clone(), ty.name.clone()), index)
                    })
                })
                .collect();
        Self {
            image,
            builds,
            slots,
            type_indices,
        }
    }

    fn addr(&self, package_id: &str, name: &str) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Package(self.slots[package_id]),
            file: FileAddr::LoadedFileIndex(0),
            type_index: self.type_indices[&(package_id.to_string(), name.to_string())],
        }
    }

    fn executable_addr(&self, package_id: &str) -> ExecutableAddr {
        ExecutableAddr {
            unit: UnitAddr::Package(self.slots[package_id]),
            file: FileAddr::LoadedFileIndex(0),
            executable: 0,
        }
    }

    fn export_context<'a>(
        &'a self,
        projection: &'a RuntimeAssemblyExecutionProjection,
        provider_heap: &'a RequestHeap,
        provider: &str,
        caller: Option<&str>,
    ) -> ServiceErrorExportContext<'a> {
        ServiceErrorExportContext {
            execution_image: &self.image,
            type_view: projection.type_view(),
            provider_heap,
            provider_package_build_id: &self.builds[provider],
            caller_package_build_id: caller.map(|package| &self.builds[package]),
            provider_service_id: "svc.provider",
            operation_id: "errors.throw",
        }
    }

    fn import(
        &self,
        projection: &RuntimeAssemblyExecutionProjection,
        error: OpaqueServiceError,
        caller: &str,
        heap: &mut RequestHeap,
        site: &InstructionSourceSite,
    ) -> Result<UserException> {
        let stack = [ExceptionStackFrame::Local { site: site.clone() }];
        CanonicalServiceErrorChannel::import_caller_failure(
            error,
            ServiceErrorImportContext {
                execution_image: &self.image,
                type_view: projection.type_view(),
                caller_heap: heap,
                caller_package_build_id: &self.builds[caller],
                caller_executable_addr: &self.executable_addr(caller),
                call_site: site,
                caller_stack_at_site: &stack,
                remote_service_id: "svc.provider",
                remote_operation_id: "errors.throw",
            },
        )
    }
}

struct PublicType {
    name: String,
    source: TypeDescriptorIr,
    schema: Option<ContractTypeDescriptor>,
    type_params: Vec<String>,
}

impl PublicType {
    fn public(name: &str, source: TypeDescriptorIr, schema: ContractTypeDescriptor) -> Self {
        Self {
            name: name.to_string(),
            source,
            schema: Some(schema),
            type_params: Vec::new(),
        }
    }

    fn private(name: &str, source: TypeDescriptorIr, type_params: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            source,
            schema: None,
            type_params,
        }
    }
}

struct PackageFixture {
    package_id: String,
    artifact_ref: PackageArtifactRef,
    artifact: Arc<PackageArtifact>,
    index: Arc<PackageSchemaIndex>,
    records: BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>,
    file: Arc<FileIrUnit>,
    types: Vec<PublicType>,
}

fn package(package_id: &str, types: Vec<PublicType>) -> PackageFixture {
    let mut records = BTreeMap::new();
    let mut index_types = BTreeMap::new();
    for ty in &types {
        let Some(schema) = &ty.schema else {
            continue;
        };
        let canonical_descriptor = PackageSchemaCanonicalDescriptor {
            type_params: ty.type_params.clone(),
            descriptor: schema.clone(),
        };
        let type_id = skiff_artifact_identity::package_schema_type_id(
            package_id,
            &ty.name,
            &canonical_descriptor,
        )
        .expect("fixture type identity");
        records.insert(
            type_id.clone(),
            Arc::new(PackageSchemaTypeRecord {
                package_id: package_id.to_string(),
                stable_schema_key: ty.name.clone(),
                package_schema_type_id: type_id.clone(),
                canonical_descriptor,
            }),
        );
        index_types.insert(
            ty.name.clone(),
            PackageSchemaIndexEntry {
                package_schema_type_id: type_id,
                public_path: Some(ty.name.clone()),
                nameability: ContractTypeNameability::PublicNameable,
            },
        );
    }
    let index = Arc::new(PackageSchemaIndex {
        package_id: package_id.to_string(),
        package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
            package_id,
            &index_types,
        )
        .expect("fixture schema index identity"),
        types: index_types,
    });
    let mut file = FileIrUnit::empty("errors", format!("source:{package_id}"));
    file.type_table = types
        .iter()
        .map(|ty| ArtifactTypeDecl {
            name: ty.name.clone(),
            descriptor: ty.source.clone(),
            type_params: ty.type_params.clone(),
            implements: Vec::new(),
            source_span: None,
        })
        .collect();
    skiff_artifact_identity::assign_file_ir_identity(&mut file).expect("fixture file identity");
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let build = PackageBuildId::new(format!("build:{package_id}"));
    let local_abi = PackageLocalAbiIdentity::new(format!("abi:{package_id}"));
    let implementation_types = types
        .iter()
        .enumerate()
        .filter(|(_, ty)| ty.schema.is_some())
        .map(|(type_index, ty)| {
            (
                ty.name.clone(),
                TypeExport {
                    file: file_ref.clone(),
                    type_index: u32::try_from(type_index).expect("fixture type index fits u32"),
                    symbol: ty.name.clone(),
                    is_interface: false,
                    descriptor: Some(ty.source.clone()),
                    type_params: ty.type_params.clone(),
                    interface_methods: Vec::new(),
                },
            )
        })
        .collect();
    let artifact = Arc::new(PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: build.clone(),
        files: vec![file_ref],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: local_abi.clone(),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: index.package_schema_index_identity.clone(),
        },
        package_schema_type_records: records
            .values()
            .map(|record| {
                (
                    record.package_schema_type_id.clone(),
                    PackageSchemaTypeRecordRef {
                        package_id: package_id.to_string(),
                        package_schema_type_id: record.package_schema_type_id.clone(),
                    },
                )
            })
            .collect(),
        implementation_links: PackageImplementationLinks {
            types: implementation_types,
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    });
    PackageFixture {
        package_id: package_id.to_string(),
        artifact_ref: PackageArtifactRef {
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: build,
            package_local_abi_identity: local_abi,
        },
        artifact,
        index,
        records,
        file: Arc::new(file),
        types,
    }
}
