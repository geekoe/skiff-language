use std::sync::Arc;

use skiff_runtime_linked_program::{
    BlockIr, CallIr, ExecutableAddr, ExecutableKind, ExprRefIr, ExternalRefTable, FileAddr,
    FileDeclarations, FileLinkTargets, LinkOverlay, LinkedCallTarget, LinkedExecutable,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedStmtIr, PublicationResourceTable,
    RuntimeTypeContext, SlotLayoutIr, SourceMapDto, StmtRefIr, UnitAddr,
};
use skiff_runtime_model::request_heap::RequestHeap;

use super::*;
use crate::{env::Env, EvalRuntimeProgram, Interpreter};

struct TailCallFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    caller: ExecutableAddr,
}

impl TailCallFixture {
    fn new() -> Self {
        let file_id = "file:r1-tail-call-execution".to_string();
        let caller = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(file_id.clone()),
            executable: 0,
        };
        let callee = ExecutableAddr {
            executable: 1,
            ..caller.clone()
        };
        let site = site();
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: file_id,
            source_ast_hash: "source:r1-tail-call-execution".to_string(),
            module_path: "r1.tailCall".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![
                executable(
                    "caller",
                    vec![LinkedExprIr::Call {
                        call: CallIr {
                            target: LinkedCallTarget::Executable { addr: callee },
                            site,
                            args: Vec::new(),
                            type_args: Default::default(),
                            metadata: Default::default(),
                            actor_metadata: None,
                        },
                    }],
                ),
                executable(
                    "callee",
                    vec![LinkedExprIr::Literal {
                        value: skiff_artifact_model::LiteralIr::Null,
                    }],
                ),
            ],
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/r1-tail-call-execution",
            vec![Arc::clone(&file)],
            Vec::new(),
            PublicationResourceTable::default(),
            Default::default(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        Self {
            interpreter: Interpreter::with_program(program, test_runtime::runtime_factory()),
            file,
            caller,
        }
    }

    async fn execute_callable(
        &self,
        context: ProgramExecutionContext<'static>,
    ) -> Result<crate::env::Flow, RuntimeError> {
        let mut heap = RequestHeap::default();
        let mut env = Env::for_program_executable(
            &self.file.executables[0],
            Some(self.file.module_path.clone()),
            0,
        )?;
        self.interpreter
            .exec_program_executable(
                context,
                &mut heap,
                &mut env,
                &self.caller,
                &self.file,
                &self.file.executables[0],
            )
            .await
    }

    async fn execute_barrier(
        &self,
        context: ProgramExecutionContext<'static>,
    ) -> Result<crate::env::Flow, RuntimeError> {
        let mut heap = RequestHeap::default();
        let mut env = Env::for_program_executable(
            &self.file.executables[0],
            Some(self.file.module_path.clone()),
            0,
        )?;
        self.interpreter
            .exec_program_block(
                context,
                &mut heap,
                &mut env,
                &self.caller,
                &self.file,
                &self.file.executables[0],
                "entry",
            )
            .await
    }
}

fn executable(symbol: &str, expressions: Vec<LinkedExprIr>) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            expressions,
        },
    }
}

fn max_depth_context() -> ProgramExecutionContext<'static> {
    let (cancellation, scope) = root_scope(None);
    context(ScopeAwareControl::available(scope, cancellation.token()))
        .with_program_call_depth_for_test(super::super::MAX_PROGRAM_CALL_DEPTH)
}

#[tokio::test]
async fn tail_call_shared_trampoline_does_not_push_non_tail_depth() {
    let fixture = TailCallFixture::new();
    let flow = fixture
        .execute_callable(max_depth_context())
        .await
        .expect("tail transfer must replace the active frame");
    assert!(matches!(flow, crate::env::Flow::Return(_)));
}

#[tokio::test]
async fn tail_call_lexical_barrier_falls_back_to_an_ordinary_call() {
    let fixture = TailCallFixture::new();
    assert!(
        fixture.execute_barrier(max_depth_context()).await.is_err(),
        "barrier evaluation must retain the ordinary nested-call depth push"
    );
}
