use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use skiff_artifact_model::{
    BlockIr, DbDeclarationIr as ArtifactDbDeclarationIr, DbFieldStorageIr, DbObjectFieldIr,
    DbObjectKeyIr as ArtifactDbObjectKeyIr, DbObjectKindIr as ArtifactDbObjectKindIr,
    ExecutableBody, ExecutableIr, ExecutableKind, ExprIr, LiteralIr, SlotLayout, StmtIr, StmtRefIr,
    TypeDeclIr, TypeDeclarationIr as ArtifactTypeDeclarationIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityContextApi, DbCapabilityFuture, DbCapabilityLeaseHandle,
    DbCapabilityLeaseHold, DbCapabilityResult, DbCapabilityStore, DbCapabilityStoreApi,
    DbCapabilityTarget, DbCapabilityTargetId, DbDocument, DbKey, DbOneSelector, DbOrderEntry,
    DbPageResult, DbQuery, DbRecoverableRuntimeContext, DbRuntimeChange, DbRuntimeFinalizer,
    DbWriteResult, FieldPath, FileCapabilityRecord, PreparedDbValueRuntimeOperation,
    ServiceDbChange, ServiceDbFindOptions,
};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, DbBodyIr, DbChangeIr, DbChangeOpIr, DbObjectTargetId, DbOpKindIr,
    DbOperationIr, DbSelectorIr, DbTargetIr, ExecutableAddr, ExprRefIr, FieldPathIr, FileAddr,
    LinkedExecutable, LinkedFileUnit, LinkedTypeRef, TypeAddr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

use super::*;
use crate::{
    error::RuntimeError, program_execution::ProgramExecutionInput, DbContractBinding, Env,
    Interpreter,
};

fn json_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Json".to_string(),
        args: Vec::new(),
    }
}

fn engine_file() -> FileIrUnit {
    let mut file = FileIrUnit::empty("engine.main", "source:db-contract-view-engine");
    file.file_ir_identity = "file:db-contract-view-engine".to_string();
    file.type_table.push(TypeDeclIr {
        name: "AgentThread".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("string")),
                ("status".to_string(), TypeRefIr::builtin("string")),
            ]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        "AgentThread".to_string(),
        ArtifactTypeDeclarationIr {
            type_index: 0,
            symbol: "engine.main.AgentThread".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "AgentThread".to_string(),
        ArtifactDbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index: 0 },
            type_name: "AgentThread".to_string(),
            collection_name: None,
            implements: None,
            identity_fields: BTreeMap::new(),
            kind: ArtifactDbObjectKindIr::Contract,
            key: ArtifactDbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![DbObjectFieldIr {
                name: "status".to_string(),
                ty: TypeRefIr::builtin("string"),
                storage: DbFieldStorageIr::Identity,
            }],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "engineEntry".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("Json"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(skiff_artifact_model::ExprRefIr { expression: 1 }),
            }],
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "t1".to_string(),
                    },
                },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "closed".to_string(),
                    },
                },
            ],
        },
        source_span: None,
    });
    file
}

fn host_file() -> FileIrUnit {
    let mut file = FileIrUnit::empty("host.main", "source:db-contract-view-host");
    file.file_ir_identity = "file:db-contract-view-host".to_string();
    file.type_table.push(TypeDeclIr {
        name: "Thread".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::from([
                ("id".to_string(), TypeRefIr::builtin("string")),
                ("status".to_string(), TypeRefIr::builtin("string")),
                ("hostOnly".to_string(), TypeRefIr::builtin("string")),
            ]),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        "Thread".to_string(),
        ArtifactTypeDeclarationIr {
            type_index: 0,
            symbol: "host.main.Thread".to_string(),
            source_span: None,
        },
    );
    file.declarations.db.insert(
        "Thread".to_string(),
        ArtifactDbDeclarationIr {
            type_ref: TypeRefIr::LocalType { type_index: 0 },
            type_name: "Thread".to_string(),
            collection_name: Some("threads".to_string()),
            implements: None,
            identity_fields: BTreeMap::new(),
            kind: ArtifactDbObjectKindIr::Object,
            key: ArtifactDbObjectKeyIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
            },
            fields: vec![
                DbObjectFieldIr {
                    name: "status".to_string(),
                    ty: TypeRefIr::builtin("string"),
                    storage: DbFieldStorageIr::Identity,
                },
                DbObjectFieldIr {
                    name: "hostOnly".to_string(),
                    ty: TypeRefIr::builtin("string"),
                    storage: DbFieldStorageIr::Identity,
                },
            ],
            retention: None,
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    file
}

#[derive(Default)]
struct ContractViewStoreState {
    lookup_keys: Vec<String>,
    rows: BTreeMap<String, DbDocument>,
    transaction_events: Vec<&'static str>,
    changes: Vec<ServiceDbChange>,
}

struct ContractViewFixture {
    image: Arc<AssemblyExecutionImage>,
    activation: Arc<ActivationContext>,
    caller_addr: ExecutableAddr,
    contract_target: DbObjectTargetId,
    host_target: DbObjectTargetId,
    store_state: Arc<Mutex<ContractViewStoreState>>,
}

impl ContractViewFixture {
    fn host_lookup_key(&self) -> String {
        DbCapabilityTarget::new(
            DbCapabilityTargetId {
                package_artifact_ref: self.host_target.package_artifact_ref.clone(),
                file_ir_ref: self.host_target.file_ir_ref.clone(),
                type_index: self.host_target.type_index,
            },
            "Thread".to_string(),
        )
        .lookup_key()
        .to_string()
    }

    fn into_eval_target(self, binding: Option<Arc<DbContractBinding>>) -> ContractViewExecution {
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> =
            Arc::new(TestContractBindingResolver {
                activation: Arc::clone(&self.activation),
                binding,
            });
        let request = RequestActivationContext::begin(Arc::clone(&self.activation))
            .expect("contract view request generation should begin");
        let eval_target =
            RuntimeAssemblyEvalTarget::new(Arc::clone(&self.image), request, resolver)
                .expect("contract view image and activation should form an eval target");
        contract_view_execution(self, eval_target)
    }
}

fn build_contract_view_fixture() -> ContractViewFixture {
    let mut engine_file = engine_file();
    let mut host_file = host_file();
    skiff_artifact_identity::assign_file_ir_identity(&mut engine_file)
        .expect("engine File IR should receive a canonical identity");
    skiff_artifact_identity::assign_file_ir_identity(&mut host_file)
        .expect("host File IR should receive a canonical identity");
    let engine_file_ref = file_ref(&engine_file);
    let host_file_ref = file_ref(&host_file);

    let mut engine_package = private_package("example.db-contract-view.engine", &engine_file);
    engine_package.files = vec![engine_file_ref.clone()];
    skiff_artifact_identity::assign_package_artifact_identities(&mut engine_package)
        .expect("engine package should receive canonical identities");
    let engine_package_ref = package_ref(&engine_package);

    let mut host_package = private_package("example.db-contract-view.host", &host_file);
    host_package.files = vec![host_file_ref.clone()];
    skiff_artifact_identity::assign_package_artifact_identities(&mut host_package)
        .expect("host package should receive canonical identities");
    let host_package_ref = package_ref(&host_package);

    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:db-contract-view"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![engine_package_ref.clone(), host_package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![
                PackageCodeSlot {
                    package: engine_package_ref.clone(),
                },
                PackageCodeSlot {
                    package: host_package_ref.clone(),
                },
            ],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image = crate::test_support::link_package_fixture(
        assembly.clone(),
        vec![
            (engine_package, vec![engine_file]),
            (host_package, vec![host_file]),
        ],
    );
    let caller_addr = ExecutableAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        executable: 0,
    };
    let activation = activation_context(
        assembly.assembly_identity,
        engine_package_ref.package_build_id.clone(),
    );
    ContractViewFixture {
        image,
        activation,
        caller_addr,
        contract_target: DbObjectTargetId {
            package_artifact_ref: engine_package_ref.clone(),
            file_ir_ref: engine_file_ref.clone(),
            type_index: 0,
        },
        host_target: DbObjectTargetId {
            package_artifact_ref: host_package_ref,
            file_ir_ref: host_file_ref,
            type_index: 0,
        },
        store_state: Arc::new(Mutex::new(ContractViewStoreState::default())),
    }
}

struct TestContractBindingResolver {
    activation: Arc<ActivationContext>,
    binding: Option<Arc<DbContractBinding>>,
}

impl RuntimeAssemblyEvalResolver for TestContractBindingResolver {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id() == activation_id).then(|| Arc::clone(&self.activation))
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id().as_str() == activation_id)
            .then(|| Arc::clone(&self.activation))
    }

    fn contract(&self, _contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        None
    }

    fn admitted_schema_records(
        &self,
        _contract: &ServiceContractRef,
    ) -> Option<crate::AdmittedPackageSchemaRecords> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &ContractOperationId,
    ) -> Option<OperationTargetRef> {
        None
    }

    fn db_contract_binding(
        &self,
        contract_target: &DbObjectTargetId,
    ) -> Option<Arc<DbContractBinding>> {
        self.binding
            .as_ref()
            .filter(|binding| binding.contract == *contract_target)
            .cloned()
    }
}

#[derive(Clone)]
struct ContractViewDbStore {
    state: Arc<Mutex<ContractViewStoreState>>,
}

impl ContractViewDbStore {
    fn record_lookup(&self, type_name: &str) {
        self.state
            .lock()
            .unwrap()
            .lookup_keys
            .push(type_name.to_string());
    }

    fn unexpected(&self, method: &str) -> ! {
        panic!("unexpected DB method on contract view store: {method}")
    }
}

impl DbCapabilityStoreApi for ContractViewDbStore {
    fn begin_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        self.state.lock().unwrap().transaction_events.push("begin");
        Box::pin(async { Ok(()) })
    }

    fn commit_transaction(&self) -> DbCapabilityFuture<'_, ()> {
        self.state.lock().unwrap().transaction_events.push("commit");
        Box::pin(async { Ok(()) })
    }

    fn abort_transaction(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.state.lock().unwrap().transaction_events.push("abort");
        Box::pin(async {})
    }

    fn find_one_by_key<'a>(
        &'a self,
        type_name: &'a str,
        key: DbKey,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.record_lookup(type_name);
        let state = Arc::clone(&self.state);
        let key_text = key.as_value().as_str().map(str::to_string);
        Box::pin(async move {
            let state = state.lock().unwrap();
            Ok(key_text
                .as_deref()
                .and_then(|key| state.rows.get(key).cloned()))
        })
    }

    fn create<'a>(
        &'a self,
        _type_name: &'a str,
        value: DbDocument,
    ) -> DbCapabilityFuture<'a, DbDocument> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut state = state.lock().unwrap();
            if let Some(key) = value
                .as_value()
                .get("id")
                .and_then(serde_json::Value::as_str)
            {
                state.rows.insert(key.to_string(), value.clone());
            }
            Ok(value)
        })
    }

    fn update_one<'a>(
        &'a self,
        type_name: &'a str,
        _selector: DbOneSelector,
        change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.record_lookup(type_name);
        self.state.lock().unwrap().changes.push(change);
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let state = state.lock().unwrap();
            Ok(state.rows.values().next().cloned())
        })
    }

    fn find_one_by_key_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("find_one_by_key_runtime")
    }

    fn find_one_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("find_one_by_query")
    }

    fn find_one_by_query_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _order: Vec<DbOrderEntry>,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("find_one_by_query_runtime")
    }

    fn find_many_page<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
    ) -> DbCapabilityFuture<'a, DbPageResult> {
        self.unexpected("find_many_page")
    }

    fn find_many_page_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _options: ServiceDbFindOptions,
        _projection: Option<Vec<FieldPath>>,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Vec<RuntimeValue>> {
        self.unexpected("find_many_page_runtime")
    }

    fn create_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _value: &'a RuntimeValue,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, RuntimeValue> {
        self.unexpected("create_runtime")
    }

    fn prepare_create_runtime(
        &self,
        _type_name: &str,
        _value: &RuntimeValue,
        _heap: &mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityResult<PreparedDbValueRuntimeOperation> {
        self.unexpected("prepare_create_runtime")
    }

    fn insert_many_result<'a>(
        &'a self,
        _type_name: &'a str,
        _values: Vec<DbDocument>,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("insert_many_result")
    }

    fn update_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _change: DbRuntimeChange,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("update_one_runtime")
    }

    fn update_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("update_many")
    }

    fn upsert_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _insert: DbDocument,
        _change: ServiceDbChange,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("upsert_by_key")
    }

    fn replace_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: DbDocument,
    ) -> DbCapabilityFuture<'a, Option<DbDocument>> {
        self.unexpected("replace_one")
    }

    fn replace_one_runtime<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
        _value: &'a RuntimeValue,
        _heap: &'a mut RequestHeap,
        _context: DbRecoverableRuntimeContext,
    ) -> DbCapabilityFuture<'a, Option<RuntimeValue>> {
        self.unexpected("replace_one_runtime")
    }

    fn delete_one<'a>(
        &'a self,
        _type_name: &'a str,
        _selector: DbOneSelector,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("delete_one")
    }

    fn delete_many<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, DbWriteResult> {
        self.unexpected("delete_many")
    }

    fn count<'a>(&'a self, _type_name: &'a str, _query: DbQuery) -> DbCapabilityFuture<'a, u64> {
        self.unexpected("count")
    }

    fn exists_by_key<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("exists_by_key")
    }

    fn exists_by_query<'a>(
        &'a self,
        _type_name: &'a str,
        _query: DbQuery,
    ) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("exists_by_query")
    }

    fn claim_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<DbCapabilityLeaseHandle>> {
        self.unexpected("claim_lease")
    }

    fn renew_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, bool> {
        self.unexpected("renew_lease")
    }

    fn release_lease<'a>(&'a self, _hold: &'a DbCapabilityLeaseHold) -> DbCapabilityFuture<'a, ()> {
        self.unexpected("release_lease")
    }

    fn read_lease<'a>(
        &'a self,
        _type_name: &'a str,
        _key: DbKey,
        _slot: &'a str,
    ) -> DbCapabilityFuture<'a, Option<serde_json::Value>> {
        self.unexpected("read_lease")
    }

    fn lease_lost(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        self.unexpected("lease_lost")
    }

    fn insert_skiff_file_record<'a>(
        &'a self,
        _record: FileCapabilityRecord,
    ) -> DbCapabilityFuture<'a, ()> {
        self.unexpected("insert_skiff_file_record")
    }

    fn find_skiff_file_by_id<'a>(
        &'a self,
        _id: &'a str,
    ) -> DbCapabilityFuture<'a, Option<FileCapabilityRecord>> {
        self.unexpected("find_skiff_file_by_id")
    }

    fn delete_skiff_file_by_id<'a>(&'a self, _id: &'a str) -> DbCapabilityFuture<'a, u64> {
        self.unexpected("delete_skiff_file_by_id")
    }
}

#[derive(Clone)]
struct ContractViewDbContext {
    store: DbCapabilityStore,
}

impl DbCapabilityContextApi for ContractViewDbContext {
    fn require_store(
        &self,
        _target: &str,
        _unavailable_reason: &str,
    ) -> DbCapabilityResult<DbCapabilityStore> {
        Ok(self.store.clone())
    }
}

struct ContractViewExecution {
    interpreter: Interpreter,
    context: ProgramExecutionContext<'static>,
    addr: ExecutableAddr,
    file: Arc<LinkedFileUnit>,
    executable: Arc<LinkedExecutable>,
    contract_target: DbObjectTargetId,
    host_lookup_key: String,
    store_state: Arc<Mutex<ContractViewStoreState>>,
}

fn contract_view_execution(
    fixture: ContractViewFixture,
    eval_target: RuntimeAssemblyEvalTarget,
) -> ContractViewExecution {
    let host_lookup_key = fixture.host_lookup_key();
    let contract_target = fixture.contract_target.clone();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let projection = eval_target.execution_projection();
    let addr = fixture.caller_addr;
    let file = projection
        .resolve_file(&UnitAddr::Package(0), &FileAddr::LoadedFileIndex(0))
        .expect("engine file should resolve")
        .clone();
    let executable = Arc::new(
        projection
            .resolve_executable(&addr)
            .expect("engine executable should resolve")
            .executable
            .clone(),
    );
    let store = DbCapabilityStore::new(ContractViewDbStore {
        state: Arc::clone(&fixture.store_state),
    });
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let test_effect_doubles = interpreter.test_effect_double_context();
    let actor = test_runtime::actor_context();
    let request = test_runtime::request_context();
    let stream_runtime = interpreter.stream_runtime.clone();
    let rebinder = test_runtime::activation_execution_context_rebinder(
        &actor,
        &request,
        stream_runtime.clone(),
        test_effect_doubles.clone(),
        interpreter.http_options.clone(),
    );
    let context = ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: DbCapabilityContext::new(ContractViewDbContext { store }),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(stream_runtime.clone()),
        time: skiff_runtime_capability_context::TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            stream_runtime,
            test_effect_doubles.clone(),
        ),
        test_effect_doubles,
        actor,
        request,
        request_heap_limits: RequestHeapLimits::default(),
    })
    .with_activation_execution_context_rebinder(rebinder)
    .with_runtime_assembly_target(eval_target);
    ContractViewExecution {
        interpreter,
        context,
        addr,
        file,
        executable,
        contract_target,
        host_lookup_key,
        store_state: fixture.store_state,
    }
}

fn contract_target_operation(
    contract_target: &DbObjectTargetId,
    op: DbOpKindIr,
    change: Option<DbChangeIr>,
    record_result: bool,
) -> DbOperationIr {
    DbOperationIr {
        op,
        many: false,
        target: DbTargetIr {
            target_id: contract_target.clone(),
            type_ref: json_type(),
            type_name: "AgentThread".to_string(),
        },
        selector: Some(DbSelectorIr::Key {
            value: ExprRefIr { expression: 0 },
        }),
        query: None,
        projection: None,
        body: Some(DbBodyIr::ObjectFields {
            fields: BTreeMap::new(),
        }),
        insert_body: Some(DbBodyIr::ObjectFields {
            fields: BTreeMap::new(),
        }),
        change,
        result_type: if record_result {
            LinkedTypeRef::Address {
                addr: TypeAddr {
                    unit: UnitAddr::Package(0),
                    file: FileAddr::LoadedFileIndex(0),
                    type_index: 0,
                },
            }
        } else {
            json_type()
        },
        source_span: None,
    }
}

async fn eval_operation(
    execution: &mut ContractViewExecution,
    operation: &DbOperationIr,
) -> Result<(RuntimeValue, RequestHeap), RuntimeError> {
    let heap = RequestHeap::default();
    let mut access = HeapAccess::private(heap);
    let result = execution
        .interpreter
        .eval_program_db_operation(
            execution.context.clone(),
            &mut access,
            &mut Env::new(),
            &execution.addr,
            &execution.file,
            &execution.executable,
            operation,
        )
        .await;
    let heap = access.into_owned_heap();
    result.map(|value| (value, heap))
}

fn bound_execution() -> ContractViewExecution {
    let fixture = build_contract_view_fixture();
    let binding = Arc::new(DbContractBinding::new(
        fixture.contract_target.clone(),
        fixture.host_target.clone(),
    ));
    fixture.into_eval_target(Some(binding))
}

fn unbound_execution() -> ContractViewExecution {
    build_contract_view_fixture().into_eval_target(None)
}

#[tokio::test]
async fn contract_view_reads_resolve_to_host_collection_and_ignore_undeclared_fields() {
    let mut execution = bound_execution();
    execution.store_state.lock().unwrap().rows.insert(
        "t1".to_string(),
        DbDocument::new(serde_json::json!({
            "id": "t1",
            "status": "open",
            "hostOnly": "host-wrote-this",
        })),
    );
    let operation =
        contract_target_operation(&execution.contract_target, DbOpKindIr::Optional, None, true);
    let (value, heap) = eval_operation(&mut execution, &operation)
        .await
        .expect("contract view find must resolve to the host collection");
    let state = execution.store_state.lock().unwrap();
    assert_eq!(
        state.lookup_keys,
        vec![execution.host_lookup_key.clone()],
        "the store must see the host collection key, not a contract key"
    );
    drop(state);

    let wire = crate::runtime_ops::runtime_to_wire(&value, &heap)
        .expect("contract view result should convert to wire value");
    assert_eq!(wire["id"], serde_json::json!("t1"));
    assert_eq!(wire["status"], serde_json::json!("open"));
    assert!(
        wire.get("hostOnly").is_none(),
        "fields the contract view does not declare must be ignored, got {wire}"
    );
}

#[tokio::test]
async fn contract_view_field_scoped_update_only_touches_declared_engine_fields() {
    let mut execution = bound_execution();
    execution.store_state.lock().unwrap().rows.insert(
        "t1".to_string(),
        DbDocument::new(serde_json::json!({
            "id": "t1",
            "status": "open",
            "hostOnly": "host-wrote-this",
        })),
    );
    let operation = contract_target_operation(
        &execution.contract_target,
        DbOpKindIr::Update,
        Some(DbChangeIr {
            ops: vec![DbChangeOpIr::Set {
                field: FieldPathIr {
                    text: "status".to_string(),
                    segments: vec!["status".to_string()],
                },
                value: ExprRefIr { expression: 1 },
            }],
        }),
        false,
    );
    eval_operation(&mut execution, &operation)
        .await
        .expect("contract view field-scoped update must execute on the host collection");
    let state = execution.store_state.lock().unwrap();
    assert_eq!(
        state.lookup_keys,
        vec![execution.host_lookup_key.clone()],
        "the store must see the host collection key for updates"
    );
    assert_eq!(state.changes.len(), 1);
    assert_eq!(
        state.changes[0].ops()[0].field(),
        "status",
        "the engine change must only name contract-declared fields"
    );
}

#[tokio::test]
async fn contract_view_engine_initialization_runs_inside_host_transaction_scope() {
    let mut execution = bound_execution();
    let store = {
        let state = Arc::clone(&execution.store_state);
        DbCapabilityStore::new(ContractViewDbStore { state })
    };
    store
        .begin_transaction()
        .await
        .expect("host transaction begin");
    store
        .create(
            "threads",
            DbDocument::new(serde_json::json!({
                "id": "t1",
                "status": "open",
                "hostOnly": "host-wrote-this",
            })),
        )
        .await
        .expect("host insert must succeed");
    let operation =
        contract_target_operation(&execution.contract_target, DbOpKindIr::Optional, None, true);
    eval_operation(&mut execution, &operation)
        .await
        .expect("engine initialization must run inside the host transaction");
    store
        .commit_transaction()
        .await
        .expect("host transaction commit");
    let state = execution.store_state.lock().unwrap();
    assert_eq!(
        state.transaction_events,
        vec!["begin", "commit"],
        "engine contract-view calls must participate in the host transaction"
    );
    assert_eq!(
        state.lookup_keys,
        vec![execution.host_lookup_key.clone()],
        "engine reads inside the host transaction must hit the host collection key"
    );
}

#[tokio::test]
async fn contract_view_insert_replace_and_upsert_fail_closed() {
    for (op, label) in [
        (DbOpKindIr::Insert, "insert"),
        (DbOpKindIr::Replace, "replace"),
        (DbOpKindIr::Upsert, "upsert"),
    ] {
        let mut execution = bound_execution();
        let operation = contract_target_operation(&execution.contract_target, op, None, false);
        let error = eval_operation(&mut execution, &operation)
            .await
            .expect_err(&format!("contract view {label} must fail closed"));
        assert!(
            error.to_string().contains("contract target"),
            "{label}: {error}"
        );
        let state = execution.store_state.lock().unwrap();
        assert!(
            state.lookup_keys.is_empty(),
            "{label}: the store must never be reached for rejected writes"
        );
    }
}

#[tokio::test]
async fn unbound_contract_target_fails_closed_at_resolution() {
    let mut execution = unbound_execution();
    let operation =
        contract_target_operation(&execution.contract_target, DbOpKindIr::Optional, None, true);
    let error = eval_operation(&mut execution, &operation)
        .await
        .expect_err("a contract target without a host binding must fail closed");
    assert!(
        error.to_string().contains("no host implementation binding"),
        "{error}"
    );
}
