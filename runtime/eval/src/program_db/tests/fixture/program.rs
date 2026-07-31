use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use skiff_artifact_model::{
    ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity, FileIrRef,
    InstructionSourceSite, LiteralIr, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
    SyntheticInstructionSiteReason, ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_runtime_linked_program::{
    linked::{DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr, DbObjectKindIr, TypeDeclarationIr},
    BlockIr, CallIr, DbBodyIr, DbLeaseClaimIr, DbLeaseReadIr, DbObjectTargetId, DbOpKindIr,
    DbOperationIr, DbQueryIr, DbTargetIr, DbTransactionIr, DbTransactionModeIr, ExecutableAddr,
    ExecutableKind, ExprRefIr, ExternalRefTable, FileAddr, FileDeclarations, FileLinkTargets,
    LinkOverlay, LinkedActorDeclaration, LinkedActorDeclarationOwner, LinkedActorField,
    LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit,
    LinkedInterfaceInstantiationRef, LinkedStmtIr, LinkedTypeRef, PublicationResourceTable,
    RuntimeTypeContext, ServiceSymbolRef, SlotIr, SlotLayoutIr, SourceMapDto, StmtRefIr, UnitAddr,
};

use crate::{actor_executor_test_runtime as test_runtime, EvalRuntimeProgram, Interpreter};

pub(in crate::program_db::tests) const FILE_ID: &str = "file:db-actor-fixture";
pub(in crate::program_db::tests) const ACTOR_SERVICE_ID: &str = "skiff.run/db-actor-fixture";
pub(in crate::program_db::tests) const ACTOR_TYPE_ID: &str = "svc.main.CheckpointActor";
pub(in crate::program_db::tests) const BODY_CREATE_BLOCK_LABEL: &str = "body-create";
pub(in crate::program_db::tests) const ILLEGAL_FLOW_BLOCK_LABEL: &str = "illegal-flow";
pub(in crate::program_db::tests) const TAIL_CALL_BARRIER_BLOCK_LABEL: &str = "tail-call-barrier";
const DB_PACKAGE_ID: &str = "skiff.run/db-actor-fixture-package";
const DB_PACKAGE_VERSION: &str = "1.0.0";
const DB_PACKAGE_BUILD: &str = "build:db-actor-fixture";
const DB_PACKAGE_ABI: &str = "abi:db-actor-fixture";

pub(in crate::program_db::tests) struct LinkedDbActorFixture {
    pub program: Arc<EvalRuntimeProgram>,
    pub interpreter: Interpreter,
    pub file: Arc<LinkedFileUnit>,
    pub addr: ExecutableAddr,
    pub raw_create: DbOperationIr,
    pub prepared_create: DbOperationIr,
    pub query_target: DbTargetIr,
    pub query: DbQueryIr,
    pub legacy_transaction: CallIr,
    pub explicit_transaction: DbTransactionIr,
    pub claim: DbLeaseClaimIr,
    pub read: DbLeaseReadIr,
    pub exact_local_call: CallIr,
}

impl LinkedDbActorFixture {
    pub(in crate::program_db::tests) fn new() -> Self {
        let ir = fixture_ir();
        let file = linked_file(&ir);
        let package = crate::test_support::runtime_execution_package_fixture_with_identity(
            DB_PACKAGE_ID,
            DB_PACKAGE_VERSION,
            DB_PACKAGE_BUILD,
            DB_PACKAGE_ABI,
            0,
            vec![Arc::clone(&file)],
            PublicationResourceTable::default(),
        );
        let addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
            executable: 0,
        };
        let program = Arc::new(EvalRuntimeProgram::new(
            ACTOR_SERVICE_ID,
            vec![Arc::clone(&file)],
            vec![package],
            PublicationResourceTable::default(),
            HashMap::new(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        let interpreter =
            Interpreter::with_program(Arc::clone(&program), test_runtime::runtime_factory());
        Self {
            program,
            interpreter,
            file,
            addr,
            raw_create: ir.raw_create,
            prepared_create: ir.prepared_create,
            query_target: ir.query_target,
            query: ir.query,
            legacy_transaction: ir.legacy_transaction,
            explicit_transaction: ir.explicit_transaction,
            claim: ir.claim,
            read: ir.read,
            exact_local_call: ir.exact_local_call,
        }
    }

    pub(in crate::program_db::tests) fn executable(&self) -> &LinkedExecutable {
        self.file
            .executables
            .first()
            .expect("DB/Actor fixture executable")
    }
}

struct FixtureIr {
    raw_create: DbOperationIr,
    prepared_create: DbOperationIr,
    query_target: DbTargetIr,
    query: DbQueryIr,
    legacy_transaction: CallIr,
    explicit_transaction: DbTransactionIr,
    claim: DbLeaseClaimIr,
    read: DbLeaseReadIr,
    exact_local_call: CallIr,
}

fn fixture_ir() -> FixtureIr {
    FixtureIr {
        raw_create: raw_create(),
        prepared_create: prepared_create(),
        query_target: thread_target(),
        query: DbQueryIr::default(),
        legacy_transaction: legacy_transaction(),
        explicit_transaction: explicit_transaction(),
        claim: lease_claim(),
        read: lease_read(),
        exact_local_call: exact_local_call(),
    }
}

pub(in crate::program_db::tests) fn actor_owner() -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        actor_symbol: "CheckpointActor".to_string(),
    }
}

pub(in crate::program_db::tests) fn actor_abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:db-actor-fixture")
}

pub(in crate::program_db::tests) fn actor_implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:db-actor-fixture")
}

pub(in crate::program_db::tests) fn integer_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "integer".to_string(),
        args: Vec::new(),
    }
}

fn json_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Json".to_string(),
        args: Vec::new(),
    }
}

fn string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "String".to_string(),
        args: Vec::new(),
    }
}

fn runtime_string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

fn thread_type() -> LinkedTypeRef {
    db_object_type("Thread")
}

fn raw_thread_type() -> LinkedTypeRef {
    db_object_type("RawThread")
}

fn db_object_type(symbol: &str) -> LinkedTypeRef {
    LinkedTypeRef::DbObjectSymbol {
        symbol: ServiceSymbolRef {
            module_path: "svc.main".to_string(),
            symbol: symbol.to_string(),
        },
    }
}

fn plain_target() -> DbTargetIr {
    DbTargetIr {
        target_id: db_target_id(1),
        type_ref: json_type(),
        type_name: "RawThread".to_string(),
    }
}

fn thread_target() -> DbTargetIr {
    DbTargetIr {
        target_id: db_target_id(0),
        type_ref: thread_type(),
        type_name: "Thread".to_string(),
    }
}

fn raw_create() -> DbOperationIr {
    DbOperationIr {
        op: DbOpKindIr::Insert,
        many: false,
        target: plain_target(),
        selector: None,
        query: None,
        projection: None,
        body: Some(DbBodyIr::ObjectFields {
            fields: BTreeMap::new(),
        }),
        insert_body: None,
        change: None,
        result_type: json_type(),
        source_span: None,
    }
}

fn prepared_create() -> DbOperationIr {
    DbOperationIr {
        op: DbOpKindIr::Insert,
        many: false,
        target: thread_target(),
        selector: None,
        query: None,
        projection: None,
        body: Some(DbBodyIr::ObjectFields {
            fields: BTreeMap::new(),
        }),
        insert_body: None,
        change: None,
        result_type: json_type(),
        source_span: None,
    }
}

fn legacy_transaction() -> CallIr {
    CallIr {
        target: LinkedCallTarget::Builtin {
            op: "db.transaction".to_string(),
        },
        site: synthetic_site(),
        args: vec![ExprRefIr { expression: 0 }],
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

fn explicit_transaction() -> DbTransactionIr {
    DbTransactionIr {
        mode: DbTransactionModeIr::Value,
        body: "empty".to_string(),
        result: Some(ExprRefIr { expression: 0 }),
        result_type: json_type(),
    }
}

fn lease_claim() -> DbLeaseClaimIr {
    DbLeaseClaimIr {
        target: thread_target(),
        key: ExprRefIr { expression: 1 },
        slot: "owner".to_string(),
        binding_slot: Some(0),
        body: "empty".to_string(),
        result_type: LinkedTypeRef::Native {
            name: "boolean".to_string(),
            args: Vec::new(),
        },
        source_span: None,
    }
}

fn lease_read() -> DbLeaseReadIr {
    DbLeaseReadIr {
        target: thread_target(),
        key: ExprRefIr { expression: 1 },
        slot: "owner".to_string(),
        result_type: json_type(),
        source_span: None,
    }
}

fn exact_local_call() -> CallIr {
    CallIr {
        target: LinkedCallTarget::Executable {
            addr: ExecutableAddr {
                unit: UnitAddr::Service,
                file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
                executable: 0,
            },
        },
        site: synthetic_site(),
        args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn linked_file(ir: &FixtureIr) -> Arc<LinkedFileUnit> {
    let mut declarations = FileDeclarations::default();
    declarations.types.insert(
        "Thread".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Thread".to_string(),
            source_span: None,
        },
    );
    declarations.types.insert(
        "RawThread".to_string(),
        TypeDeclarationIr {
            type_index: 1,
            symbol: "RawThread".to_string(),
            source_span: None,
        },
    );
    declarations.db.insert(
        "Thread".to_string(),
        DbDeclarationIr {
            type_ref: thread_type(),
            type_name: "Thread".to_string(),
            collection_name: "Thread".to_string(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: string_type(),
            },
            fields: vec![DbObjectFieldIr {
                name: "runtime".to_string(),
                ty: LinkedTypeRef::AnyInterface {
                    interface: LinkedInterfaceInstantiationRef {
                        interface_abi_id: "fixture.Runtime".to_string(),
                        canonical_type_args: Vec::new(),
                    },
                },
                storage: Default::default(),
            }],
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    declarations.db.insert(
        "RawThread".to_string(),
        DbDeclarationIr {
            type_ref: raw_thread_type(),
            type_name: "RawThread".to_string(),
            collection_name: "RawThread".to_string(),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: string_type(),
            },
            fields: Vec::new(),
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: FILE_ID.to_string(),
        source_ast_hash: "source:db-actor-fixture".to_string(),
        module_path: "svc.main".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations,
        link_targets: FileLinkTargets::default(),
        actor_declarations: vec![LinkedActorDeclaration {
            actor_type: ServiceSymbolRef {
                module_path: "svc.main".to_string(),
                symbol: "CheckpointActor".to_string(),
            },
            implementation_owner: Some(actor_owner()),
            actor_abi_identity: actor_abi(),
            actor_implementation_identity: actor_implementation(),
            actor_name: "CheckpointActor".to_string(),
            actor_id_type: runtime_string_type(),
            key_field: "id".to_string(),
            fields: vec![
                LinkedActorField {
                    name: "id".to_string(),
                    ty: runtime_string_type(),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
                LinkedActorField {
                    name: "count".to_string(),
                    ty: integer_type(),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
            ],
            create: None,
            public_methods: Vec::new(),
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        }],
        types: vec![
            skiff_runtime_linked_program::anonymous_type_decl(
                "Thread",
                skiff_runtime_linked_program::LinkedTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            ),
            skiff_runtime_linked_program::anonymous_type_decl(
                "RawThread",
                skiff_runtime_linked_program::LinkedTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            ),
        ],
        constants: Vec::new(),
        executables: vec![LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "fixture".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(json_type()),
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "lease-binding".to_string(),
                    kind: "local".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: true,
            body: LinkedExecutableBody {
                blocks: vec![
                    BlockIr {
                        label: "entry".to_string(),
                        statements: vec![StmtRefIr { statement: 3 }],
                    },
                    BlockIr {
                        label: "empty".to_string(),
                        statements: Vec::new(),
                    },
                    BlockIr {
                        label: BODY_CREATE_BLOCK_LABEL.to_string(),
                        statements: vec![StmtRefIr { statement: 0 }],
                    },
                    BlockIr {
                        label: ILLEGAL_FLOW_BLOCK_LABEL.to_string(),
                        statements: vec![StmtRefIr { statement: 1 }],
                    },
                    BlockIr {
                        label: TAIL_CALL_BARRIER_BLOCK_LABEL.to_string(),
                        statements: vec![StmtRefIr { statement: 2 }],
                    },
                ],
                statements: vec![
                    LinkedStmtIr::Expr {
                        value: ExprRefIr { expression: 2 },
                    },
                    LinkedStmtIr::Return {
                        value: Some(ExprRefIr { expression: 0 }),
                    },
                    LinkedStmtIr::Return {
                        value: Some(ExprRefIr { expression: 9 }),
                    },
                    LinkedStmtIr::Return {
                        value: Some(ExprRefIr { expression: 10 }),
                    },
                ],
                expressions: vec![
                    LinkedExprIr::Literal {
                        value: LiteralIr::Null,
                    },
                    LinkedExprIr::Literal {
                        value: LiteralIr::String {
                            value: "thread-1".to_string(),
                        },
                    },
                    LinkedExprIr::DbOperation {
                        operation: ir.raw_create.clone(),
                    },
                    LinkedExprIr::DbOperation {
                        operation: ir.prepared_create.clone(),
                    },
                    LinkedExprIr::DbQuery {
                        target: ir.query_target.clone(),
                        query: ir.query.clone(),
                        projection: None,
                        result_type: Some(json_type()),
                    },
                    LinkedExprIr::Call {
                        call: ir.legacy_transaction.clone(),
                    },
                    LinkedExprIr::DbTransaction {
                        transaction: ir.explicit_transaction.clone(),
                    },
                    LinkedExprIr::DbLeaseClaim {
                        claim: ir.claim.clone(),
                    },
                    LinkedExprIr::DbLeaseRead {
                        read: ir.read.clone(),
                    },
                    LinkedExprIr::Call {
                        call: ir.exact_local_call.clone(),
                    },
                    LinkedExprIr::ArrayLiteral {
                        items: vec![ExprRefIr { expression: 11 }],
                    },
                    LinkedExprIr::Literal {
                        value: LiteralIr::String {
                            value: "structured-tail-result".to_string(),
                        },
                    },
                ],
            },
        }],
        external_refs: ExternalRefTable::default(),
    })
}

fn db_file_ref() -> FileIrRef {
    FileIrRef {
        file_ir_identity: FILE_ID.to_string(),
        module_path: "svc.main".to_string(),
        artifact_path: None,
        source_ast_hash: Some("source:db-actor-fixture".to_string()),
    }
}

fn db_target_id(type_index: usize) -> DbObjectTargetId {
    DbObjectTargetId {
        package_artifact_ref: PackageArtifactRef {
            package_id: DB_PACKAGE_ID.to_string(),
            package_version: DB_PACKAGE_VERSION.to_string(),
            package_build_id: PackageBuildId::new(DB_PACKAGE_BUILD),
            package_local_abi_identity: PackageLocalAbiIdentity::new(DB_PACKAGE_ABI),
        },
        file_ir_ref: db_file_ref(),
        type_index,
    }
}
