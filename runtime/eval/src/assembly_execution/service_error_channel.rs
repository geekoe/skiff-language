//! Canonical fixed service-failure export and import.
//!
//! Boundary lanes only provide typed execution facts to this module. They do
//! not classify, flatten, or infer service errors themselves.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use skiff_artifact_model::{
    ContractTypeRef, InstructionSourceSite, PackageBuildId, PackageSchemaIndexEntry, SourceSpanRef,
    TypeExport,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_boundary::{
    payload::{PayloadBoundary, PayloadBoundaryKind},
    service_value_plan::ServiceValuePlan,
    ServiceValueSelection,
};
use skiff_runtime_capability_context::{
    RestrictedServiceDiagnostic, RestrictedServiceDiagnosticCauseKind,
    RestrictedServiceDiagnosticOwner, TelemetryCapabilityContext,
};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, ExecutableAddr, FileAddr, PackageCodeSlotIndex,
    ServiceErrorDeclarationKind, ServiceErrorExecutionContext, ServiceErrorExecutionKey,
    ServiceErrorPublicIdentity, ServiceErrorTypeLink, SharedPackageCode, TypeAddr, UnitAddr,
};
use skiff_runtime_linked_type_plan::ProgramTypeView;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, InternalErrorPayload,
        LocalExecutionTypeIdentity, NamedUnionOwnerIdentity, NominalTypeIdentity,
        OpaqueServiceError, PlatformBuiltinErrorIdentity, RequestException, ServiceErrorEnvelope,
    },
};

use crate::{
    error::{
        decode_opaque_service_error, diagnostic_source_frames, Result, RuntimeError, UserException,
    },
    exceptions::{
        exact_named_union_branch_index, materialize_service_error_local_value,
        user_exception_for_catch,
    },
    runtime_ops::{runtime_from_wire, runtime_to_wire},
};

/// The only public-safe message emitted for provider-local failures that
/// cannot be represented as a public or platform service error.
pub(crate) const INTERNAL_SERVICE_ERROR_MESSAGE: &str = "Internal service error";

const INTERNAL_ERROR_PACKAGE_ID: &str = "skiff.run/std";
const INTERNAL_ERROR_STABLE_SCHEMA_KEY: &str = "std.service.InternalError";
const CHANNEL_TARGET: &str = "canonical service error channel";

/// Stateless owner of the canonical service-error export/import protocol.
#[derive(Debug, Default)]
pub(crate) struct CanonicalServiceErrorChannel;

/// Exact execution and boundary facts required while the provider heap is alive.
pub(crate) struct ServiceErrorExportContext<'a> {
    pub(crate) execution_image: &'a AssemblyExecutionImage,
    pub(crate) type_view: ProgramTypeView<'a>,
    pub(crate) provider_heap: &'a RequestHeap,
    pub(crate) provider_package_build_id: &'a PackageBuildId,
    /// Present for an in-process caller and absent for ingress. It is a typed
    /// provenance fact, never an input to error classification.
    pub(crate) caller_package_build_id: Option<&'a PackageBuildId>,
    pub(crate) provider_service_id: &'a str,
    pub(crate) operation_id: &'a str,
}

/// Exact caller-local facts required to import one already-fixed failure.
pub(crate) struct ServiceErrorImportContext<'a> {
    pub(crate) execution_image: &'a AssemblyExecutionImage,
    pub(crate) type_view: ProgramTypeView<'a>,
    pub(crate) caller_heap: &'a mut RequestHeap,
    pub(crate) caller_package_build_id: &'a PackageBuildId,
    pub(crate) caller_executable_addr: &'a ExecutableAddr,
    pub(crate) call_site: &'a InstructionSourceSite,
    /// The caller-local stack including `call_site`, obtained from the request
    /// context before crossing the remote boundary.
    pub(crate) caller_stack_at_site: &'a [ExceptionStackFrame],
    pub(crate) remote_service_id: &'a str,
    pub(crate) remote_operation_id: &'a str,
}

/// Provider-local facts used only for the restricted diagnostic sidecar.
pub(crate) struct RestrictedServiceDiagnosticExportContext<'a> {
    pub(crate) telemetry: &'a TelemetryCapabilityContext,
    pub(crate) provider_activation_id: &'a str,
    pub(crate) request_generation: u64,
    pub(crate) fallback_source: &'a InstructionSourceSite,
    pub(crate) fallback_stack: &'a [ExceptionStackFrame],
}

impl CanonicalServiceErrorChannel {
    /// Fixes one provider failure and submits its provider-local diagnostic.
    ///
    /// Diagnostic submission is best-effort and cannot change the fixed result.
    /// The provider heap remains borrowed through both export and submission.
    pub(crate) fn export_provider_failure_with_diagnostic(
        actual_error: &RuntimeError,
        context: ServiceErrorExportContext<'_>,
        diagnostic_context: RestrictedServiceDiagnosticExportContext<'_>,
        next_correlation: impl FnOnce() -> Result<ErrorCorrelation>,
    ) -> Result<OpaqueServiceError> {
        let provider_service_id = context.provider_service_id.to_string();
        let operation_id = context.operation_id.to_string();
        let fixed = Self::export_provider_failure(actual_error, context, next_correlation)?;
        let (source, stack) = user_exception_for_catch(actual_error)
            .map(|exception| {
                (
                    exception.request().source().clone(),
                    exception.request().stack().to_vec(),
                )
            })
            .or_else(|| {
                let stack = diagnostic_instruction_stack(actual_error);
                let source = stack.last().and_then(|frame| match frame {
                    ExceptionStackFrame::Local { site } => Some(site.clone()),
                    ExceptionStackFrame::RemoteBoundary { .. } => None,
                })?;
                Some((source, stack))
            })
            .unwrap_or_else(|| {
                (
                    diagnostic_context.fallback_source.clone(),
                    diagnostic_context.fallback_stack.to_vec(),
                )
            });
        let cause_kind = match fixed.envelope() {
            ServiceErrorEnvelope::PublicTypedError { .. } => {
                RestrictedServiceDiagnosticCauseKind::PublicTypedError
            }
            ServiceErrorEnvelope::InternalError { .. } => {
                RestrictedServiceDiagnosticCauseKind::InternalError
            }
            ServiceErrorEnvelope::PlatformError { .. } => {
                RestrictedServiceDiagnosticCauseKind::PlatformError
            }
        };
        let diagnostic = RestrictedServiceDiagnostic {
            owner: RestrictedServiceDiagnosticOwner {
                provider_service_id,
                operation_id,
                provider_activation_id: diagnostic_context.provider_activation_id.to_string(),
                request_generation: diagnostic_context.request_generation,
            },
            correlation: ErrorCorrelation {
                trace_id: fixed.envelope().trace_id().to_string(),
                error_id: fixed.envelope().error_id().to_string(),
            },
            source,
            stack,
            cause_kind,
        };
        let _ = diagnostic_context
            .telemetry
            .submit_restricted_service_diagnostic(&diagnostic);
        Ok(fixed)
    }

    /// Fixes one actual provider failure before its heap can be dropped.
    ///
    /// `next_correlation` is invoked only for a newly-created platform or
    /// Internal cause. Local and imported exceptions retain their correlation.
    pub(crate) fn export_provider_failure(
        actual_error: &RuntimeError,
        context: ServiceErrorExportContext<'_>,
        next_correlation: impl FnOnce() -> Result<ErrorCorrelation>,
    ) -> Result<OpaqueServiceError> {
        if actual_error.is_cancellation_terminal() {
            return Err(RuntimeError::Cancelled);
        }
        if let Some(error) = actual_error.fixed_service_failure() {
            return Ok(error.clone());
        }

        let actual_error = crate::error::unwrap_diagnostic_source_context(actual_error);
        if let Some(exception) = user_exception_for_catch(actual_error) {
            if let Some(error) = exception.request().fixed_service_error() {
                return Ok(error.clone());
            }
            validate_export_context(&context)?;
            return export_local_exception(exception, &context);
        }

        validate_export_context(&context)?;
        if let RuntimeError::InvalidArtifact(message) = actual_error {
            return Err(RuntimeError::InvalidArtifact(message.clone()));
        }

        if let Some((identity, payload)) = actual_error.ordinary_catch_projection() {
            if let Some(identity) = platform_identity(&identity) {
                let correlation = next_correlation()?;
                return match encode_platform_payload(identity, &payload) {
                    Ok(encoded_payload) => fixed_error(ServiceErrorEnvelope::PlatformError {
                        builtin_error_identity: identity,
                        encoded_payload,
                        trace_id: correlation.trace_id,
                        error_id: correlation.error_id,
                    }),
                    Err(_) => fixed_internal(correlation),
                };
            }
        }

        fixed_internal(next_correlation()?)
    }

    /// Imports one strict fixed failure into a fresh caller-local exception.
    pub(crate) fn import_caller_failure(
        error: OpaqueServiceError,
        mut context: ServiceErrorImportContext<'_>,
    ) -> Result<UserException> {
        validate_import_context(&context, &error)?;
        let local_value = match error.envelope() {
            ServiceErrorEnvelope::PublicTypedError {
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                ..
            } => import_public_error(
                package_id,
                stable_schema_key,
                package_schema_type_id,
                encoded_payload,
                &mut context,
            )?,
            ServiceErrorEnvelope::InternalError { payload } => {
                Some(import_internal_error(payload, &mut context)?)
            }
            ServiceErrorEnvelope::PlatformError {
                builtin_error_identity,
                encoded_payload,
                ..
            } => Some(import_platform_error(
                *builtin_error_identity,
                encoded_payload,
                context.caller_heap,
            )?),
        };

        let mut stack = context.caller_stack_at_site.to_vec();
        stack.push(ExceptionStackFrame::RemoteBoundary {
            service_id: context.remote_service_id.to_string(),
            operation_id: context.remote_operation_id.to_string(),
            error_id: error.envelope().error_id().to_string(),
        });
        let request =
            RequestException::imported(error, local_value, context.call_site.clone(), stack)
                .map_err(RuntimeError::InvalidArtifact)?;
        Ok(UserException::new(request))
    }
}

fn diagnostic_instruction_stack(error: &RuntimeError) -> Vec<ExceptionStackFrame> {
    diagnostic_source_frames(error)
        .into_iter()
        .filter_map(|frame| {
            // Keep only the typed span reference. Diagnostic frame paths,
            // functions, messages, and other open-ended values stay out.
            frame
                .get("span")
                .and_then(|span| span.get("span"))
                .and_then(|span| serde_json::from_value::<SourceSpanRef>(span.clone()).ok())
                .map(|span| ExceptionStackFrame::Local {
                    site: InstructionSourceSite::Source { span },
                })
        })
        .collect()
}

#[cfg(test)]
pub(crate) struct RecordingRestrictedServiceDiagnosticSink;

#[cfg(test)]
static RESTRICTED_SERVICE_DIAGNOSTIC_PROBES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::BTreeMap<u64, Vec<RestrictedServiceDiagnostic>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));

#[cfg(test)]
impl skiff_runtime_capability_context::RestrictedServiceDiagnosticSink
    for RecordingRestrictedServiceDiagnosticSink
{
    fn submit(
        &self,
        diagnostic: &RestrictedServiceDiagnostic,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        if let Ok(mut probes) = RESTRICTED_SERVICE_DIAGNOSTIC_PROBES.lock() {
            if let Some(records) = probes.get_mut(&diagnostic.owner.request_generation) {
                records.push(diagnostic.clone());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn start_restricted_service_diagnostic_probe_for_test(request_generation: u64) {
    if let Ok(mut probes) = RESTRICTED_SERVICE_DIAGNOSTIC_PROBES.lock() {
        probes.insert(request_generation, Vec::new());
    }
}

#[cfg(test)]
pub(crate) fn take_restricted_service_diagnostics_for_test(
    request_generation: u64,
) -> Vec<RestrictedServiceDiagnostic> {
    RESTRICTED_SERVICE_DIAGNOSTIC_PROBES
        .lock()
        .ok()
        .and_then(|mut probes| probes.remove(&request_generation))
        .unwrap_or_default()
}

fn export_local_exception(
    exception: &UserException,
    context: &ServiceErrorExportContext<'_>,
) -> Result<OpaqueServiceError> {
    let request = exception.request();
    let carrier = request.local_value().ok_or_else(|| {
        RuntimeError::InvalidArtifact(
            "local service exception is missing its exact local value".to_string(),
        )
    })?;
    let identity = carrier.catch_identity().ok_or_else(|| {
        RuntimeError::InvalidArtifact(
            "local service exception is missing its exact catch identity".to_string(),
        )
    })?;
    if matches!(
        identity,
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(identity))
            if !admitted_platform_identity(*identity)
    ) {
        return Err(RuntimeError::InvalidArtifact(
            "local service exception uses an unadmitted platform identity".to_string(),
        ));
    }

    if let Some(identity) = platform_identity(identity) {
        let payload = match runtime_to_wire(carrier.value(), context.provider_heap)
            .ok()
            .and_then(|payload| encode_platform_payload(identity, &payload).ok())
        {
            Some(payload) => payload,
            None => return fixed_internal(request.correlation().clone()),
        };
        return fixed_error(ServiceErrorEnvelope::PlatformError {
            builtin_error_identity: identity,
            encoded_payload: payload,
            trace_id: request.correlation().trace_id.clone(),
            error_id: request.correlation().error_id.clone(),
        });
    }

    let (key, selection) =
        local_service_error_key(identity, context.execution_image, context.type_view)?;
    let Some((key, selection)) = key.zip(selection) else {
        return fixed_internal(request.correlation().clone());
    };
    let Some(link) = context
        .execution_image
        .service_error_types()
        .by_execution(&key)
    else {
        if public_artifact_identity_for_addr(context.execution_image, key.execution_addr())?
            .is_some()
        {
            return Err(RuntimeError::InvalidArtifact(
                "public schema-closed service error is missing its execution index row".to_string(),
            ));
        }
        return fixed_internal(request.correlation().clone());
    };
    validate_export_link(context, link)?;

    if is_internal_error_identity(link.public_identity()) {
        return fixed_internal(request.correlation().clone());
    }

    let encoded_payload = match with_service_value_plan(
        context.execution_image,
        link,
        context.provider_service_id,
        |plan, boundary| {
            validate_plan_selection(plan, link.context(), selection)?;
            Ok(plan
                .encode_binary_selected(carrier.value(), selection, boundary, context.provider_heap)
                .ok())
        },
    )? {
        Some(payload) => payload,
        None => return fixed_internal(request.correlation().clone()),
    };
    let identity = link.public_identity();
    fixed_error(ServiceErrorEnvelope::PublicTypedError {
        package_id: identity.package_id().to_string(),
        stable_schema_key: identity.stable_schema_key().to_string(),
        package_schema_type_id: identity.package_schema_type_id().clone(),
        encoded_payload,
        trace_id: request.correlation().trace_id.clone(),
        error_id: request.correlation().error_id.clone(),
    })
}

fn local_service_error_key(
    identity: &CatchIdentity,
    image: &AssemblyExecutionImage,
    type_view: ProgramTypeView<'_>,
) -> Result<(
    Option<ServiceErrorExecutionKey>,
    Option<ServiceValueSelection>,
)> {
    match identity {
        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(local)) => {
            if !local_identity_is_closed(local, image)? {
                return Ok((None, None));
            }
            let declaration = image.types().declaration(&local.addr).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "service error local type address {} is not interned",
                    local.addr
                ))
            })?;
            if matches!(
                declaration.descriptor,
                skiff_runtime_linked_program::LinkedTypeDescriptor::Union { .. }
                    | skiff_runtime_linked_program::LinkedTypeDescriptor::Alias { .. }
                    | skiff_runtime_linked_program::LinkedTypeDescriptor::Interface
            ) {
                return Err(RuntimeError::InvalidArtifact(
                    "service error nominal identity does not name a record or representation"
                        .to_string(),
                ));
            }
            Ok((
                Some(ServiceErrorExecutionKey::Declaration {
                    addr: local.addr.clone(),
                }),
                Some(ServiceValueSelection::Root),
            ))
        }
        CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::LocalExecution(local),
            branch,
        } => {
            if !local_identity_is_closed(local, image)? {
                return Ok((None, None));
            }
            let branch_index = exact_named_union_branch_index(local, branch, type_view)?;
            Ok((
                Some(ServiceErrorExecutionKey::NamedUnionBranch {
                    union_addr: local.addr.clone(),
                    branch_index,
                }),
                Some(ServiceValueSelection::NamedUnionBranch(branch_index)),
            ))
        }
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(_)) => {
            Err(RuntimeError::InvalidArtifact(
                "platform identity reached Package service-error lookup".to_string(),
            ))
        }
        CatchIdentity::Nominal(NominalTypeIdentity::PackageSchema(_))
        | CatchIdentity::NamedUnionBranch {
            union: NamedUnionOwnerIdentity::PackageSchema(_),
            ..
        } => Err(RuntimeError::InvalidArtifact(
            "provider-local exception carried a Package-schema identity instead of an execution identity"
                .to_string(),
        )),
    }
}

fn local_identity_is_closed(
    identity: &LocalExecutionTypeIdentity,
    image: &AssemblyExecutionImage,
) -> Result<bool> {
    let declaration = image.types().declaration(&identity.addr).ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "service error local type address {} is not interned",
            identity.addr
        ))
    })?;
    if declaration.type_params.len() != identity.type_arguments.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service error local type {} expects {} type argument(s), got {}",
            identity.addr,
            declaration.type_params.len(),
            identity.type_arguments.len()
        )));
    }
    Ok(identity.type_arguments.is_empty())
}

fn import_public_error(
    package_id: &str,
    stable_schema_key: &str,
    package_schema_type_id: &skiff_artifact_model::PackageSchemaTypeId,
    encoded_payload: &[u8],
    context: &mut ServiceErrorImportContext<'_>,
) -> Result<Option<RuntimeValueCarrier>> {
    let identity = ServiceErrorPublicIdentity::new(
        package_id,
        stable_schema_key,
        package_schema_type_id.clone(),
    );
    let Some(candidates) = context
        .execution_image
        .service_error_types()
        .by_public_identity(&identity)
    else {
        if assembly_contains_package(context.execution_image, package_id) {
            return Err(protocol(format!(
                "public service error identity conflicts with admitted Package {package_id}"
            )));
        }
        return Ok(None);
    };

    let linked = exact_graph_candidates(
        context.execution_image,
        context.caller_package_build_id,
        &identity,
        candidates,
    )?;
    if linked.is_empty() {
        return Ok(None);
    }
    let code_slots = linked
        .iter()
        .map(|link| execution_code_slot(link.context()))
        .collect::<Result<BTreeSet<_>>>()?;
    if code_slots.len() != 1 {
        return Err(protocol(
            "caller graph selected more than one build for a public service error",
        ));
    }

    let representative = linked[0];
    ensure_link_group_consistent(&linked)?;
    let decoded = with_service_value_plan(
        context.execution_image,
        representative,
        context.remote_service_id,
        |plan, boundary| {
            plan.decode_binary_selected(encoded_payload, boundary, context.caller_heap)
                .map_err(|error| {
                    protocol(format!("public service error payload rejected: {error}"))
                })
        },
    )?;
    let selected = select_decoded_link(&linked, decoded.selection)?;
    with_service_value_plan(
        context.execution_image,
        selected,
        context.remote_service_id,
        |plan, _| {
            validate_plan_selection(plan, selected.context(), decoded.selection)
                .map_err(|error| protocol(error.to_string()))
        },
    )?;
    let value = materialize_service_error_local_value(
        decoded.value,
        selected,
        context.type_view,
        context.caller_executable_addr,
        context.caller_heap,
    )?;
    Ok(Some(value))
}

fn import_internal_error(
    payload: &InternalErrorPayload,
    context: &mut ServiceErrorImportContext<'_>,
) -> Result<RuntimeValueCarrier> {
    if payload.message != INTERNAL_SERVICE_ERROR_MESSAGE {
        return Err(protocol(
            "InternalError payload does not contain the canonical fixed message",
        ));
    }
    let identities = context
        .execution_image
        .service_error_types()
        .public_identities()
        .filter(|identity| is_internal_error_identity(identity))
        .cloned()
        .collect::<Vec<_>>();
    let [identity] = identities.as_slice() else {
        return Err(RuntimeError::InvalidArtifact(format!(
            "caller assembly must contain exactly one {INTERNAL_ERROR_STABLE_SCHEMA_KEY} public identity"
        )));
    };
    let candidates = context
        .execution_image
        .service_error_types()
        .by_public_identity(identity)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "InternalError identity disappeared from the service-error index".to_string(),
            )
        })?;
    let linked = exact_graph_candidates(
        context.execution_image,
        context.caller_package_build_id,
        identity,
        candidates,
    )?;
    let [link] = linked.as_slice() else {
        return if linked.is_empty() {
            Err(RuntimeError::InvalidArtifact(
                "caller exact package graph does not link std.service.InternalError".to_string(),
            ))
        } else {
            Err(protocol(
                "caller graph selected an ambiguous std.service.InternalError build",
            ))
        };
    };
    if !matches!(
        link.context(),
        ServiceErrorExecutionContext::Declaration {
            kind: ServiceErrorDeclarationKind::Record,
            ..
        }
    ) {
        return Err(RuntimeError::InvalidArtifact(
            "std.service.InternalError index row is not an exact record declaration".to_string(),
        ));
    }
    with_service_value_plan(
        context.execution_image,
        link,
        context.remote_service_id,
        |plan, _| validate_internal_error_plan(plan),
    )?;

    let checkpoint = context.caller_heap.checkpoint();
    let result = (|| {
        let handle = context
            .caller_heap
            .alloc_object_carriers(BTreeMap::from([
                (
                    "message".to_string(),
                    RuntimeValueCarrier::unidentified(RuntimeValue::String(
                        payload.message.clone(),
                    )),
                ),
                (
                    "traceId".to_string(),
                    RuntimeValueCarrier::unidentified(RuntimeValue::String(
                        payload.trace_id.clone(),
                    )),
                ),
                (
                    "errorId".to_string(),
                    RuntimeValueCarrier::unidentified(RuntimeValue::String(
                        payload.error_id.clone(),
                    )),
                ),
            ]))
            .map_err(RuntimeError::from)?;
        materialize_service_error_local_value(
            RuntimeValue::Heap(handle),
            link,
            context.type_view,
            context.caller_executable_addr,
            context.caller_heap,
        )
    })();
    if result.is_err() {
        context.caller_heap.rollback_to_checkpoint(checkpoint);
    }
    result
}

fn import_platform_error(
    identity: PlatformBuiltinErrorIdentity,
    encoded_payload: &[u8],
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let payload = decode_platform_payload(identity, encoded_payload).map_err(|message| {
        protocol(format!(
            "platform service error payload rejected: {message}"
        ))
    })?;
    let value = runtime_from_wire(&payload, heap)
        .map_err(|error| protocol(format!("platform service error payload rejected: {error}")))?;
    Ok(RuntimeValueCarrier::identified(
        value,
        identity.catch_identity(),
    ))
}

fn with_service_value_plan<T>(
    image: &AssemblyExecutionImage,
    link: &ServiceErrorTypeLink,
    target_service: &str,
    operation: impl FnOnce(&ServiceValuePlan<'_>, &PayloadBoundary) -> Result<T>,
) -> Result<T> {
    validate_link_record(image, link)?;
    let identity = link.public_identity();
    let contract_type = ContractTypeRef::PackageSchema {
        package_id: identity.package_id().to_string(),
        stable_schema_key: identity.stable_schema_key().to_string(),
        package_schema_type_id: identity.package_schema_type_id().clone(),
    };
    let slot = execution_code_slot(link.context())?;
    let code = image
        .shared_packages()
        .code_by_slot(PackageCodeSlotIndex::new(slot))
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service-error index points at missing package code slot {slot}"
            ))
        })?;
    let plan = ServiceValuePlan::compile(&contract_type, code.schema_records())
        .map_err(|error| RuntimeError::InvalidArtifact(error.to_string()))?;
    let boundary =
        PayloadBoundary::cross_service(PayloadBoundaryKind::ServiceResponse, target_service);
    operation(&plan, &boundary)
}

fn validate_plan_selection(
    plan: &ServiceValuePlan<'_>,
    context: &ServiceErrorExecutionContext,
    selection: ServiceValueSelection,
) -> Result<()> {
    use skiff_runtime_model::type_plan::RuntimeTypeNode;
    match (context, selection, plan.runtime_type_plan().node()) {
        (
            ServiceErrorExecutionContext::Declaration {
                kind: ServiceErrorDeclarationKind::Record,
                ..
            },
            ServiceValueSelection::Root,
            RuntimeTypeNode::Record { .. },
        )
        | (
            ServiceErrorExecutionContext::Declaration {
                kind: ServiceErrorDeclarationKind::Representation,
                ..
            },
            ServiceValueSelection::Root,
            RuntimeTypeNode::Representation { .. },
        ) => Ok(()),
        (
            ServiceErrorExecutionContext::NamedUnionBranch {
                branch_index: expected,
                ..
            },
            ServiceValueSelection::NamedUnionBranch(actual),
            RuntimeTypeNode::Union(branches),
        ) if *expected == actual && actual < branches.len() => Ok(()),
        _ => Err(RuntimeError::InvalidArtifact(
            "service-error index row, schema root, and selected codec root disagree".to_string(),
        )),
    }
}

fn validate_internal_error_plan(plan: &ServiceValuePlan<'_>) -> Result<()> {
    use skiff_runtime_model::type_plan::RuntimeTypeNode;

    let RuntimeTypeNode::Record { fields, .. } = plan.runtime_type_plan().node() else {
        return Err(RuntimeError::InvalidArtifact(
            "std.service.InternalError schema root is not a record".to_string(),
        ));
    };
    if fields.len() != 3
        || fields.iter().any(|field| {
            !matches!(field.name.as_str(), "message" | "traceId" | "errorId")
                || !field.required
                || !matches!(field.ty.node(), RuntimeTypeNode::String)
        })
    {
        return Err(RuntimeError::InvalidArtifact(
            "std.service.InternalError schema must be the exact required message/traceId/errorId string record"
                .to_string(),
        ));
    }
    Ok(())
}

fn exact_graph_candidates<'a>(
    image: &AssemblyExecutionImage,
    caller_build: &PackageBuildId,
    identity: &ServiceErrorPublicIdentity,
    candidates: &'a [std::sync::Arc<ServiceErrorTypeLink>],
) -> Result<Vec<&'a ServiceErrorTypeLink>> {
    if image
        .shared_packages()
        .code_by_build(caller_build)
        .is_none()
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "caller package build {caller_build} is not loaded"
        )));
    }
    let mut linked = Vec::new();
    for candidate in candidates {
        validate_link_record(image, candidate)?;
        let build = link_package_build(image, candidate)?;
        if build_is_exact_graph_owner(image, caller_build, identity.package_id(), build) {
            linked.push(candidate.as_ref());
        }
    }
    Ok(linked)
}

fn validate_export_link(
    context: &ServiceErrorExportContext<'_>,
    link: &ServiceErrorTypeLink,
) -> Result<()> {
    validate_link_record(context.execution_image, link)?;
    let build = link_package_build(context.execution_image, link)?;
    if !build_is_exact_graph_owner(
        context.execution_image,
        context.provider_package_build_id,
        link.public_identity().package_id(),
        build,
    ) {
        return Err(RuntimeError::InvalidArtifact(
            "provider exception identity is outside its exact Package graph".to_string(),
        ));
    }
    Ok(())
}

fn validate_link_record(image: &AssemblyExecutionImage, link: &ServiceErrorTypeLink) -> Result<()> {
    let slot = execution_code_slot(link.context())?;
    let code = image
        .shared_packages()
        .code_by_slot(PackageCodeSlotIndex::new(slot))
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service-error index points at missing package code slot {slot}"
            ))
        })?;
    let identity = link.public_identity();
    if code.artifact().package_id != identity.package_id()
        || link.record().package_id != identity.package_id()
        || link.record().stable_schema_key != identity.stable_schema_key()
        || &link.record().package_schema_type_id != identity.package_schema_type_id()
    {
        return Err(RuntimeError::InvalidArtifact(
            "service-error index owner/key/type-id record invariant is broken".to_string(),
        ));
    }
    let admitted = code
        .schema_records()
        .get(identity.package_schema_type_id())
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "service-error index record is absent from its linked Package schema closure"
                    .to_string(),
            )
        })?;
    if admitted.as_ref() != link.record().as_ref() {
        return Err(RuntimeError::InvalidArtifact(
            "service-error index record conflicts with its linked Package schema closure"
                .to_string(),
        ));
    }
    let implementation_identity =
        public_artifact_identity_for_addr(image, link.context().execution_addr())?.ok_or_else(
            || {
                RuntimeError::InvalidArtifact(
                    "service-error index execution address is not a public schema type".to_string(),
                )
            },
        )?;
    if &implementation_identity != identity {
        return Err(RuntimeError::InvalidArtifact(
            "service-error index execution address and public identity disagree".to_string(),
        ));
    }
    validate_execution_context(image, link.context())?;
    Ok(())
}

fn validate_execution_context(
    image: &AssemblyExecutionImage,
    context: &ServiceErrorExecutionContext,
) -> Result<()> {
    use skiff_runtime_linked_program::LinkedTypeDescriptor;

    let declaration = image
        .types()
        .declaration(context.execution_addr())
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "service-error index execution address is not interned".to_string(),
            )
        })?;
    match (context, &declaration.descriptor) {
        (
            ServiceErrorExecutionContext::Declaration {
                kind: ServiceErrorDeclarationKind::Record,
                ..
            },
            LinkedTypeDescriptor::Record { .. },
        )
        | (
            ServiceErrorExecutionContext::Declaration {
                kind: ServiceErrorDeclarationKind::Representation,
                ..
            },
            LinkedTypeDescriptor::Representation { .. },
        ) => Ok(()),
        (
            ServiceErrorExecutionContext::NamedUnionBranch {
                branch_index,
                branch,
                ..
            },
            LinkedTypeDescriptor::Union { branches },
        ) if branches.get(*branch_index) == Some(branch) => Ok(()),
        _ => Err(RuntimeError::InvalidArtifact(
            "service-error index execution context conflicts with its linked declaration"
                .to_string(),
        )),
    }
}

fn link_package_build<'a>(
    image: &'a AssemblyExecutionImage,
    link: &ServiceErrorTypeLink,
) -> Result<&'a PackageBuildId> {
    let slot = execution_code_slot(link.context())?;
    image
        .shared_packages()
        .code_by_slot(PackageCodeSlotIndex::new(slot))
        .map(|code| code.package_build_id())
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service-error index points at missing package code slot {slot}"
            ))
        })
}

fn build_is_exact_graph_owner(
    image: &AssemblyExecutionImage,
    caller_build: &PackageBuildId,
    owner_package_id: &str,
    candidate_build: &PackageBuildId,
) -> bool {
    if caller_build == candidate_build {
        return image
            .shared_packages()
            .code_by_build(caller_build)
            .is_some_and(|code| code.artifact().package_id == owner_package_id);
    }
    image
        .shared_packages()
        .package_link_plan()
        .package_links
        .iter()
        .any(|binding| {
            binding.key.caller_package_build_id == *caller_build
                && binding.package.package_id == owner_package_id
                && binding.package.package_build_id == *candidate_build
        })
}

fn execution_code_slot(context: &ServiceErrorExecutionContext) -> Result<usize> {
    let UnitAddr::Package(slot) = context.execution_addr().unit else {
        return Err(RuntimeError::InvalidArtifact(
            "service-error index points at a non-Package execution address".to_string(),
        ));
    };
    Ok(slot)
}

fn ensure_link_group_consistent(links: &[&ServiceErrorTypeLink]) -> Result<()> {
    let Some(first) = links.first() else {
        return Err(RuntimeError::InvalidArtifact(
            "service-error link group is empty".to_string(),
        ));
    };
    let first_slot = execution_code_slot(first.context())?;
    if links.iter().any(|link| {
        execution_code_slot(link.context()).ok() != Some(first_slot)
            || link.public_identity() != first.public_identity()
            || link.record().as_ref() != first.record().as_ref()
    }) {
        return Err(RuntimeError::InvalidArtifact(
            "service-error index rows for one caller build disagree".to_string(),
        ));
    }
    Ok(())
}

fn select_decoded_link<'a>(
    links: &'a [&ServiceErrorTypeLink],
    selection: ServiceValueSelection,
) -> Result<&'a ServiceErrorTypeLink> {
    let selected = links
        .iter()
        .copied()
        .filter(|link| match (link.context(), selection) {
            (ServiceErrorExecutionContext::Declaration { .. }, ServiceValueSelection::Root) => true,
            (
                ServiceErrorExecutionContext::NamedUnionBranch { branch_index, .. },
                ServiceValueSelection::NamedUnionBranch(selected),
            ) => *branch_index == selected,
            _ => false,
        })
        .collect::<Vec<_>>();
    match selected.as_slice() {
        [link] => Ok(*link),
        [] => Err(protocol(
            "service error payload selection has no exact caller-local index row",
        )),
        _ => Err(protocol(
            "service error payload selection has ambiguous caller-local index rows",
        )),
    }
}

fn validate_export_context(context: &ServiceErrorExportContext<'_>) -> Result<()> {
    non_empty_fact("provider service id", context.provider_service_id)?;
    non_empty_fact("operation id", context.operation_id)?;
    if context
        .execution_image
        .shared_packages()
        .code_by_build(context.provider_package_build_id)
        .is_none()
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "provider package build {} is not loaded",
            context.provider_package_build_id
        )));
    }
    if let Some(caller) = context.caller_package_build_id {
        if context
            .execution_image
            .shared_packages()
            .code_by_build(caller)
            .is_none()
        {
            return Err(RuntimeError::InvalidArtifact(format!(
                "caller package build {caller} is not loaded"
            )));
        }
    }
    Ok(())
}

fn validate_import_context(
    context: &ServiceErrorImportContext<'_>,
    error: &OpaqueServiceError,
) -> Result<()> {
    non_empty_fact("remote service id", context.remote_service_id)?;
    non_empty_fact("remote operation id", context.remote_operation_id)?;
    if context
        .caller_stack_at_site
        .iter()
        .any(|frame| matches!(frame, ExceptionStackFrame::RemoteBoundary { .. }))
    {
        return Err(RuntimeError::InvalidArtifact(
            "caller-local import stack must not contain a remote exception frame".to_string(),
        ));
    }
    if context.caller_stack_at_site.last()
        != Some(&ExceptionStackFrame::Local {
            site: context.call_site.clone(),
        })
    {
        return Err(RuntimeError::InvalidArtifact(
            "caller service-error import stack must end at the exact call site".to_string(),
        ));
    }
    error
        .envelope()
        .validate()
        .map_err(|message| protocol(format!("fixed service error is invalid: {message}")))
}

fn non_empty_fact(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{label} must be a non-empty canonical fact"
        )));
    }
    Ok(())
}

fn fixed_internal(correlation: ErrorCorrelation) -> Result<OpaqueServiceError> {
    fixed_error(ServiceErrorEnvelope::InternalError {
        payload: InternalErrorPayload {
            message: INTERNAL_SERVICE_ERROR_MESSAGE.to_string(),
            trace_id: correlation.trace_id,
            error_id: correlation.error_id,
        },
    })
}

fn fixed_error(envelope: ServiceErrorEnvelope) -> Result<OpaqueServiceError> {
    envelope.validate().map_err(RuntimeError::InvalidArtifact)?;
    let bytes = canonical_json_bytes(&envelope)?;
    decode_opaque_service_error(bytes)
}

fn is_internal_error_identity(identity: &ServiceErrorPublicIdentity) -> bool {
    identity.package_id() == INTERNAL_ERROR_PACKAGE_ID
        && identity.stable_schema_key() == INTERNAL_ERROR_STABLE_SCHEMA_KEY
}

fn platform_identity(identity: &CatchIdentity) -> Option<PlatformBuiltinErrorIdentity> {
    match identity {
        CatchIdentity::Nominal(NominalTypeIdentity::PlatformBuiltin(identity))
            if admitted_platform_identity(*identity) =>
        {
            Some(*identity)
        }
        _ => None,
    }
}

fn admitted_platform_identity(identity: PlatformBuiltinErrorIdentity) -> bool {
    matches!(
        identity,
        PlatformBuiltinErrorIdentity::Timeout
            | PlatformBuiltinErrorIdentity::ConfigDecode
            | PlatformBuiltinErrorIdentity::BytesDecode
            | PlatformBuiltinErrorIdentity::NumberDecode
            | PlatformBuiltinErrorIdentity::JsonDecode
            | PlatformBuiltinErrorIdentity::DbConflict
            | PlatformBuiltinErrorIdentity::DbDecode
            | PlatformBuiltinErrorIdentity::File
            | PlatformBuiltinErrorIdentity::TimeDecode
            | PlatformBuiltinErrorIdentity::ServiceProviderUnavailable
            | PlatformBuiltinErrorIdentity::ServiceProtocol
            | PlatformBuiltinErrorIdentity::Http
    )
}

fn assembly_contains_package(image: &AssemblyExecutionImage, package_id: &str) -> bool {
    image
        .shared_packages()
        .code_slots()
        .iter()
        .any(|code| code.artifact().package_id == package_id)
}

fn public_artifact_identity_for_addr(
    image: &AssemblyExecutionImage,
    addr: &TypeAddr,
) -> Result<Option<ServiceErrorPublicIdentity>> {
    let UnitAddr::Package(slot) = addr.unit else {
        return Err(RuntimeError::InvalidArtifact(
            "service-error execution identity points at a non-Package unit".to_string(),
        ));
    };
    let code = image
        .shared_packages()
        .code_by_slot(PackageCodeSlotIndex::new(slot))
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service-error execution identity points at missing package code slot {slot}"
            ))
        })?;
    let schema_index = code.schema_index();
    if schema_index.package_id != code.artifact().package_id
        || code.artifact().package_schema_index.package_id != schema_index.package_id
        || code
            .artifact()
            .package_schema_index
            .package_schema_index_identity
            != schema_index.package_schema_index_identity
    {
        return Err(RuntimeError::InvalidArtifact(
            "service-error Package schema index owner or identity is inconsistent".to_string(),
        ));
    }
    skiff_artifact_identity::validate_package_schema_index(schema_index).map_err(|error| {
        RuntimeError::InvalidArtifact(format!(
            "service-error Package schema index is invalid: {error}"
        ))
    })?;

    let address_file_index = exact_address_file_index(code, &addr.file)?;
    if code.files()[address_file_index]
        .type_table
        .get(addr.type_index)
        .is_none()
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service-error execution identity points at missing type index {}",
            addr.type_index
        )));
    }

    let mut matched = None;
    for (stable_schema_key, entry) in &schema_index.types {
        let public_path = entry.public_path.as_deref().ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service-error schema entry {stable_schema_key} has no exact public path"
            ))
        })?;
        let export = code
            .artifact()
            .implementation_links
            .types
            .get(public_path)
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "service-error public path {public_path} has no exact implementation type link"
                ))
            })?;
        let identity = public_schema_entry_identity(code, stable_schema_key, entry)?;
        let coordinate = exact_public_type_coordinate(code, public_path, export)?;
        if coordinate != (address_file_index, addr.type_index) {
            continue;
        }
        if matched
            .as_ref()
            .is_some_and(|first: &ServiceErrorPublicIdentity| first != &identity)
        {
            return Err(RuntimeError::InvalidArtifact(
                "service-error execution address has multiple public Package schema identities"
                    .to_string(),
            ));
        }
        matched = Some(identity);
    }
    Ok(matched)
}

fn exact_address_file_index(code: &SharedPackageCode, file_addr: &FileAddr) -> Result<usize> {
    match file_addr {
        FileAddr::LoadedFileIndex(index) => {
            code.files().get(*index).map(|_| *index).ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "service-error execution identity points at missing file index {index}"
                ))
            })
        }
        FileAddr::FileIrIdentity(identity) => {
            let mut matches = code
                .files()
                .iter()
                .enumerate()
                .filter(|(_, file)| file.file_ir_identity == *identity);
            let (index, _) = matches.next().ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!(
                    "service-error execution identity points at unloaded File IR {identity}"
                ))
            })?;
            if matches.next().is_some() {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "service-error execution identity points at ambiguous File IR {identity}"
                )));
            }
            Ok(index)
        }
    }
}

fn exact_public_type_coordinate(
    code: &SharedPackageCode,
    public_path: &str,
    export: &TypeExport,
) -> Result<(usize, usize)> {
    let mut files = code
        .files()
        .iter()
        .enumerate()
        .filter(|(_, file)| file.file_ir_identity == export.file.file_ir_identity);
    let (file_index, file) = files.next().ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "service-error public path {public_path} points at an unloaded File IR"
        ))
    })?;
    if files.next().is_some()
        || file.module_path != export.file.module_path
        || export
            .file
            .source_ast_hash
            .as_deref()
            .is_some_and(|hash| hash != file.source_ast_hash)
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service-error public path {public_path} has a non-exact File IR link"
        )));
    }
    let type_index = usize::try_from(export.type_index).map_err(|_| {
        RuntimeError::InvalidArtifact(format!(
            "service-error public path {public_path} type index does not fit the execution address space"
        ))
    })?;
    if file.type_table.get(type_index).is_none() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service-error public path {public_path} points at a missing type declaration"
        )));
    }
    Ok((file_index, type_index))
}

fn public_schema_entry_identity(
    code: &SharedPackageCode,
    stable_schema_key: &str,
    entry: &PackageSchemaIndexEntry,
) -> Result<ServiceErrorPublicIdentity> {
    let artifact_record = code
        .artifact()
        .package_schema_type_records
        .get(&entry.package_schema_type_id)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service-error schema entry {stable_schema_key} has no exact artifact record reference"
            ))
        })?;
    let record = code
        .schema_records()
        .get(&entry.package_schema_type_id)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "service-error schema entry {stable_schema_key} has no loaded record"
            ))
        })?;
    if artifact_record.package_id != code.artifact().package_id
        || artifact_record.package_schema_type_id != entry.package_schema_type_id
        || record.package_id != code.artifact().package_id
        || record.stable_schema_key != stable_schema_key
        || record.package_schema_type_id != entry.package_schema_type_id
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "service-error schema entry {stable_schema_key} owner/key/type-id invariant is broken"
        )));
    }
    Ok(ServiceErrorPublicIdentity::new(
        record.package_id.clone(),
        record.stable_schema_key.clone(),
        record.package_schema_type_id.clone(),
    ))
}

fn encode_platform_payload(
    identity: PlatformBuiltinErrorIdentity,
    value: &Value,
) -> std::result::Result<Vec<u8>, String> {
    validate_platform_payload(identity, value)?;
    canonical_json_bytes(value).map_err(|error| error.to_string())
}

fn decode_platform_payload(
    identity: PlatformBuiltinErrorIdentity,
    bytes: &[u8],
) -> std::result::Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    validate_platform_payload(identity, &value)?;
    let canonical = canonical_json_bytes(&value).map_err(|error| error.to_string())?;
    if canonical != bytes {
        return Err("payload is not the canonical encoding for its platform identity".to_string());
    }
    Ok(value)
}

fn validate_platform_payload(
    identity: PlatformBuiltinErrorIdentity,
    value: &Value,
) -> std::result::Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "platform payload must be an object".to_string())?;
    match identity {
        PlatformBuiltinErrorIdentity::File => {
            exact_fields(object, &["message"])?;
            string_field(object, "message")?;
        }
        PlatformBuiltinErrorIdentity::Timeout => {
            exact_fields(
                object,
                &["elapsedMs", "instructionCount", "limit", "reason"],
            )?;
            let reason = string_field(object, "reason")?;
            if !matches!(reason, "deadlineExceeded" | "instructionLimitExceeded") {
                return Err("timeout reason is not canonical".to_string());
            }
            u64_field(object, "instructionCount")?;
            if !object["limit"].is_null() {
                u64_field(object, "limit")?;
            }
            let elapsed = object["elapsedMs"]
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0.0)
                .ok_or_else(|| "elapsedMs must be a finite non-negative number".to_string())?;
            let _ = elapsed;
        }
        PlatformBuiltinErrorIdentity::ConfigDecode
        | PlatformBuiltinErrorIdentity::BytesDecode
        | PlatformBuiltinErrorIdentity::NumberDecode
        | PlatformBuiltinErrorIdentity::JsonDecode
        | PlatformBuiltinErrorIdentity::DbDecode
        | PlatformBuiltinErrorIdentity::TimeDecode
        | PlatformBuiltinErrorIdentity::ServiceProtocol => {
            exact_fields(object, &["message", "target"])?;
            string_field(object, "target")?;
            string_field(object, "message")?;
        }
        PlatformBuiltinErrorIdentity::DbConflict => {
            exact_fields(object, &["message", "retryable", "target"])?;
            string_field(object, "target")?;
            string_field(object, "message")?;
            if !object["retryable"].is_boolean() {
                return Err("retryable must be a boolean".to_string());
            }
        }
        PlatformBuiltinErrorIdentity::ServiceProviderUnavailable => {
            exact_fields(object, &["reason", "target"])?;
            string_field(object, "target")?;
            string_field(object, "reason")?;
        }
        PlatformBuiltinErrorIdentity::Http => {
            exact_fields(object, &["detail", "message"])?;
            string_field(object, "message")?;
        }
    }
    Ok(())
}

fn exact_fields(object: &Map<String, Value>, expected: &[&str]) -> std::result::Result<(), String> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err("platform payload fields do not match its canonical identity".to_string());
    }
    Ok(())
}

fn string_field<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> std::result::Result<&'a str, String> {
    object[field]
        .as_str()
        .ok_or_else(|| format!("{field} must be a string"))
}

fn u64_field(object: &Map<String, Value>, field: &str) -> std::result::Result<u64, String> {
    object[field]
        .as_u64()
        .ok_or_else(|| format!("{field} must be an unsigned integer"))
}

fn protocol(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Protocol {
        target: CHANNEL_TARGET.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
