use std::sync::OnceLock;
use std::{collections::HashMap, sync::Arc};

use skiff_runtime_capability_context::RequestPayloadContext;
use skiff_runtime_linked_type_plan::{PlanContext, ProgramTypeView, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::type_plan::RuntimeTypePlan;

use crate::error::{Result, RuntimeError};
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkOverlay, LinkedExecutable, LinkedExecutableBody, LinkedFileUnit,
    LinkedTypeRef, PackageUnit, PublicationResourceTable, ResolvedSymbol,
    RuntimeProgramResourceView, RuntimeTypeContext, TypeAddr, UnitAddr,
};

use super::program_ir::executable_has_explicit_self_binding;

#[derive(Clone, Copy)]
pub struct EvalExecutableBody<'a> {
    file: &'a LinkedFileUnit,
    executable: &'a LinkedExecutable,
    explicit_self_param: bool,
}

#[derive(Clone, Copy)]
pub struct EvalProgramProjection<'a> {
    pub service_id: &'a str,
    pub service_files: &'a [Arc<LinkedFileUnit>],
    pub packages: &'a [Arc<PackageUnit>],
    pub package_files: &'a [Vec<Arc<LinkedFileUnit>>],
    pub service_resources: &'a PublicationResourceTable,
    pub package_resources: &'a [PublicationResourceTable],
    pub spawn_routes: &'a HashMap<String, ExecutableAddr>,
    pub link_overlay: &'a LinkOverlay,
    pub types: &'a RuntimeTypeContext,
}

impl<'a> EvalProgramProjection<'a> {
    pub fn new(
        service_id: &'a str,
        service_files: &'a [Arc<LinkedFileUnit>],
        packages: &'a [Arc<PackageUnit>],
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
        spawn_routes: &'a HashMap<String, ExecutableAddr>,
        link_overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
    ) -> Self {
        let (service_resources, package_resources) = empty_resource_tables();
        Self::new_with_resources(
            service_id,
            service_files,
            packages,
            package_files,
            service_resources,
            package_resources,
            spawn_routes,
            link_overlay,
            types,
        )
    }

    pub fn new_with_resources(
        service_id: &'a str,
        service_files: &'a [Arc<LinkedFileUnit>],
        packages: &'a [Arc<PackageUnit>],
        package_files: &'a [Vec<Arc<LinkedFileUnit>>],
        service_resources: &'a PublicationResourceTable,
        package_resources: &'a [PublicationResourceTable],
        spawn_routes: &'a HashMap<String, ExecutableAddr>,
        link_overlay: &'a LinkOverlay,
        types: &'a RuntimeTypeContext,
    ) -> Self {
        Self {
            service_id,
            service_files,
            packages,
            package_files,
            service_resources,
            package_resources,
            spawn_routes,
            link_overlay,
            types,
        }
    }

    pub fn type_view(&self) -> ProgramTypeView<'a> {
        ProgramTypeView::new(
            self.service_files,
            self.packages,
            self.package_files,
            self.link_overlay,
            self.types,
        )
    }

    pub fn resource_view(&self) -> RuntimeProgramResourceView<'a> {
        RuntimeProgramResourceView::new(self.service_resources, self.package_resources)
    }

    pub fn resolved_service_symbol(
        &self,
        module_path: &str,
        symbol: &str,
    ) -> Option<&'a ResolvedSymbol> {
        self.link_overlay
            .resolved_service_symbol(module_path, symbol)
    }

    pub fn resolved_package_id_symbol(
        &self,
        package_id: &str,
        symbol: &str,
    ) -> Option<&'a ResolvedSymbol> {
        self.link_overlay
            .resolved_package_id_symbol(package_id, symbol)
    }

    pub fn plan_from_linked_type(
        &self,
        type_ref: &LinkedTypeRef,
        current_addr: &ExecutableAddr,
    ) -> Result<RuntimeTypePlan> {
        Ok(RuntimeTypePlan::from_linked(
            type_ref,
            &PlanContext::from_type_view(self.type_view(), current_addr),
        )?)
    }

    pub fn executable(&self, addr: &ExecutableAddr) -> Result<ResolvedEvalExecutable<'a>> {
        self.resolve_executable(addr)
    }

    pub fn resolve_file(
        &self,
        unit: &UnitAddr,
        file: &FileAddr,
    ) -> Result<&'a Arc<LinkedFileUnit>> {
        let files = self.files_for_unit(unit)?;
        match file {
            FileAddr::LoadedFileIndex(index) => files.get(*index).ok_or_else(|| {
                RuntimeError::InvalidArtifact(linked_file_index_out_of_bounds_message(
                    unit,
                    *index,
                    files.len(),
                ))
            }),
            FileAddr::FileIrIdentity(identity) => files
                .iter()
                .find(|file_unit| file_unit.file_ir_identity == *identity)
                .ok_or_else(|| {
                    RuntimeError::InvalidArtifact(linked_file_identity_not_loaded_message(
                        unit, identity,
                    ))
                }),
        }
    }

    pub fn resolve_executable(&self, addr: &ExecutableAddr) -> Result<ResolvedEvalExecutable<'a>> {
        let file_arc = self.resolve_file(&addr.unit, &addr.file)?;
        let executable = file_arc.executables.get(addr.executable).ok_or_else(|| {
            RuntimeError::InvalidArtifact(linked_executable_index_out_of_bounds_message(
                addr,
                file_arc.executables.len(),
            ))
        })?;

        Ok(ResolvedEvalExecutable {
            file: file_arc.as_ref(),
            file_arc,
            executable,
        })
    }

    pub fn executable_at(&self, addr: &ExecutableAddr) -> Result<ResolvedEvalExecutable<'a>> {
        self.resolve_executable(addr)
    }

    pub fn spawn_route(&self, target: &str) -> Option<&'a ExecutableAddr> {
        self.spawn_routes.get(target)
    }

    pub fn spawn_route_targets_for(&self, addr: &ExecutableAddr) -> Vec<&'a str> {
        self.spawn_routes
            .iter()
            .filter_map(|(target, candidate)| (candidate == addr).then_some(target.as_str()))
            .collect()
    }

    pub fn canonical_file_addr(&self, unit: &UnitAddr, file: &FileAddr) -> Result<FileAddr> {
        match file {
            FileAddr::LoadedFileIndex(_) => Ok(file.clone()),
            FileAddr::FileIrIdentity(identity) => {
                let files = self.files_for_unit(unit)?;
                let index = files
                    .iter()
                    .position(|file_unit| file_unit.file_ir_identity == *identity)
                    .ok_or_else(|| {
                        RuntimeError::InvalidArtifact(format!(
                            "RuntimeProgram file identity {identity} missing while canonicalizing type address"
                        ))
                    })?;
                Ok(FileAddr::LoadedFileIndex(index))
            }
        }
    }

    pub fn canonical_type_addr(&self, addr: &TypeAddr) -> Result<TypeAddr> {
        Ok(TypeAddr {
            unit: addr.unit.clone(),
            file: self.canonical_file_addr(&addr.unit, &addr.file)?,
            type_index: addr.type_index,
        })
    }

    fn files_for_unit(&self, unit: &UnitAddr) -> Result<&'a [Arc<LinkedFileUnit>]> {
        match unit {
            UnitAddr::Service => Ok(self.service_files),
            UnitAddr::Package(slot) => {
                self.package_files
                    .get(*slot)
                    .map(Vec::as_slice)
                    .ok_or_else(|| {
                        RuntimeError::InvalidArtifact(linked_package_slot_out_of_bounds_message(
                            *slot,
                            self.package_files.len(),
                        ))
                    })
            }
        }
    }
}

fn empty_resource_tables() -> (
    &'static PublicationResourceTable,
    &'static [PublicationResourceTable],
) {
    static EMPTY: OnceLock<(PublicationResourceTable, Vec<PublicationResourceTable>)> =
        OnceLock::new();
    let (service_resources, package_resources) =
        EMPTY.get_or_init(|| (PublicationResourceTable::default(), Vec::new()));
    (service_resources, package_resources.as_slice())
}

fn linked_package_slot_out_of_bounds_message(slot: usize, package_count: usize) -> String {
    format!("package slot {slot} out of bounds (packages: {package_count})")
}

fn linked_file_index_out_of_bounds_message(
    unit: &UnitAddr,
    index: usize,
    file_count: usize,
) -> String {
    format!("{unit} file index {index} out of bounds (files: {file_count})")
}

fn linked_file_identity_not_loaded_message(unit: &UnitAddr, identity: &str) -> String {
    format!("{unit} file identity {identity} not loaded")
}

fn linked_executable_index_out_of_bounds_message(
    addr: &ExecutableAddr,
    executable_count: usize,
) -> String {
    format!(
        "executable index {} out of bounds for {} {} (executables: {executable_count})",
        addr.executable, addr.unit, addr.file
    )
}

pub struct ResolvedEvalExecutable<'a> {
    pub file: &'a LinkedFileUnit,
    pub file_arc: &'a Arc<LinkedFileUnit>,
    pub executable: &'a LinkedExecutable,
}

impl<'a> EvalExecutableBody<'a> {
    pub fn new(file: &'a LinkedFileUnit, executable: &'a LinkedExecutable) -> Self {
        Self {
            file,
            executable,
            explicit_self_param: executable_has_explicit_self_binding(executable),
        }
    }

    pub fn file(&self) -> &'a LinkedFileUnit {
        self.file
    }

    pub fn executable(&self) -> &'a LinkedExecutable {
        self.executable
    }

    pub fn body(&self) -> &'a LinkedExecutableBody {
        &self.executable.body
    }

    pub fn explicit_self_param(&self) -> bool {
        self.explicit_self_param
    }
}

#[derive(Clone, Debug)]
pub struct BinaryHttpRequestPlan {
    pub parameter_name: String,
    pub parameter_plan: RuntimeTypePlan,
}

#[derive(Clone, Debug)]
pub struct AdapterArgPlan {
    pub parameter_name: String,
    pub source: AdapterArgSource,
    pub parameter_plan: RuntimeTypePlan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterArgSource {
    HttpRequest,
    HttpBody,
    HttpContext,
    WebSocketConnectRequest,
    WebSocketReceiveEvent,
    WebSocketConnection,
    WebSocketConnectionContext,
    WebSocketMessage,
    WebSocketMessageBody,
    WebSocketConnectionId,
    WebSocketBusinessIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpAdapterProjectionKind {
    TypedJson,
    RawHttp,
}

#[derive(Clone)]
pub struct HttpAdapterGuardProjection<'a> {
    pub invocation: Box<EvalInvocation<'a>>,
    pub request: BinaryHttpRequestPlan,
    pub response: HttpAdapterResponseProjection,
}

#[derive(Clone)]
pub struct HttpAdapterPreProjection<'a> {
    pub invocation: Box<EvalInvocation<'a>>,
    pub request: BinaryHttpRequestPlan,
}

#[derive(Clone)]
pub struct HttpAdapterProjection<'a> {
    pub kind: HttpAdapterProjectionKind,
    pub handler: Box<EvalInvocation<'a>>,
    pub handler_args: Vec<AdapterArgPlan>,
    pub guard: Option<HttpAdapterGuardProjection<'a>>,
    pub pre: Option<HttpAdapterPreProjection<'a>>,
    pub raw_handler_response: Option<HttpAdapterResponseProjection>,
}

#[derive(Clone, Debug)]
pub enum HttpAdapterResponseProjection {
    Plan(RuntimeTypePlan),
    MissingReturnType,
    InvalidHttpResponseType,
    InvalidArtifact(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebSocketAdapterProjectionKind {
    Connect,
    Receive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalWebSocketContextExpectation {
    Null,
    Typed {
        connect_operation_abi_id: String,
        context_type_identity: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalWebSocketContextCodec {
    pub operation_abi_id: String,
    pub context_type_identity: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalWebSocketConnectResult {
    Accept,
    Reject,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvalWebSocketAdapterResult {
    pub payload: Vec<u8>,
    pub response: Option<EvalWebSocketConnectResponse>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvalWebSocketConnectResponse {
    pub result: EvalWebSocketConnectResult,
    pub business_identity: Option<String>,
    pub connection_policy:
        Option<skiff_runtime_capability_context::WebSocketConnectionPolicyControl>,
    pub context_codec: Option<EvalWebSocketContextCodec>,
    pub context_payload_present: bool,
    pub code: Option<u16>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalWebSocketNameValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalWebSocketConnectRequest {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<EvalWebSocketNameValue>,
    pub headers: Vec<EvalWebSocketNameValue>,
    pub cookies: Vec<EvalWebSocketNameValue>,
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalWebSocketReceiveRequest {
    pub connection_id: String,
    pub business_identity: Option<String>,
    pub message: EvalWebSocketMessage,
    pub context_codec: Option<EvalWebSocketContextCodec>,
    pub payload_segments: Vec<EvalWebSocketPayloadSegment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalWebSocketMessage {
    pub tag: EvalWebSocketMessageTag,
    pub encoding: EvalWebSocketMessageEncoding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalWebSocketMessageTag {
    Text,
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalWebSocketMessageEncoding {
    Utf8,
    Raw,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalWebSocketPayloadSegment {
    pub kind: EvalWebSocketPayloadSegmentKind,
    pub offset: usize,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalWebSocketPayloadSegmentKind {
    Context,
    Message,
}

#[derive(Clone)]
pub struct WebSocketAdapterProjection<'a> {
    pub kind: WebSocketAdapterProjectionKind,
    pub handler: Box<EvalInvocation<'a>>,
    pub handler_args: Vec<AdapterArgPlan>,
    pub context_expectation: Option<EvalWebSocketContextExpectation>,
    pub connect_request: Option<EvalWebSocketConnectRequest>,
    pub receive_request: Option<EvalWebSocketReceiveRequest>,
}

#[derive(Clone)]
pub enum EvalBoundaryProjection<'a> {
    RuntimeUnary {
        request_payload_plan: RuntimeTypePlan,
    },
    RuntimeServerStream {
        request_payload_plan: RuntimeTypePlan,
    },
    BinaryHttpUnary {
        request: BinaryHttpRequestPlan,
    },
    BinaryHttpServerStream {
        request: BinaryHttpRequestPlan,
    },
    AdapterCallable,
    HttpAdapter {
        adapter: HttpAdapterProjection<'a>,
    },
    WebSocketAdapter {
        adapter: WebSocketAdapterProjection<'a>,
    },
}

#[derive(Clone)]
pub struct EvalInvocation<'a> {
    request: RequestPayloadContext<'a>,
    operation: &'a str,
    addr: &'a ExecutableAddr,
    program_projection: EvalProgramProjection<'a>,
    executable_body: EvalExecutableBody<'a>,
    boundary_projection: EvalBoundaryProjection<'a>,
}

impl<'a> EvalInvocation<'a> {
    pub fn new_with_projection(
        request: RequestPayloadContext<'a>,
        operation: &'a str,
        addr: &'a ExecutableAddr,
        program_projection: EvalProgramProjection<'a>,
        file: &'a LinkedFileUnit,
        executable: &'a LinkedExecutable,
        boundary_projection: EvalBoundaryProjection<'a>,
    ) -> Self {
        Self::from_parts(
            request,
            operation,
            addr,
            program_projection,
            file,
            executable,
            boundary_projection,
        )
    }

    fn from_parts(
        request: RequestPayloadContext<'a>,
        operation: &'a str,
        addr: &'a ExecutableAddr,
        program_projection: EvalProgramProjection<'a>,
        file: &'a LinkedFileUnit,
        executable: &'a LinkedExecutable,
        boundary_projection: EvalBoundaryProjection<'a>,
    ) -> Self {
        Self {
            request,
            operation,
            addr,
            program_projection,
            executable_body: EvalExecutableBody::new(file, executable),
            boundary_projection,
        }
    }

    pub fn request(&self) -> RequestPayloadContext<'a> {
        self.request.clone()
    }

    pub fn operation(&self) -> &'a str {
        self.operation
    }

    pub fn addr(&self) -> &'a ExecutableAddr {
        self.addr
    }

    pub fn program_projection(&self) -> EvalProgramProjection<'a> {
        self.program_projection
    }

    pub fn executable_body(&self) -> EvalExecutableBody<'a> {
        self.executable_body
    }

    pub fn boundary_projection(&self) -> &EvalBoundaryProjection<'a> {
        &self.boundary_projection
    }
}
