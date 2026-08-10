//! Temporary frame-diet size probe: prints `size_of` for the evaluator's key
//! context types and for the concrete future types on the non-tail recursion
//! cycle. Used to pick the biggest remaining frame contributors.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use skiff_artifact_model::{LiteralIr, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    CancellationToken, ExecutionControlApi, ExecutionControlResult, ExecutionScope,
    ExecutionScopeAccessError, FileSourceStreamContext, OwnedExecutionControl,
};
use skiff_runtime_linked_program::{
    BlockIr, CallIr, ExecutableAddr, ExecutableKind, ExprRefIr, ExternalRefTable, FileAddr,
    FileDeclarations, FileLinkTargets, LinkOverlay, LinkedCallTarget, LinkedExecutable,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedStmtIr, PublicationResourceTable,
    RuntimeTypeContext, SlotLayoutIr, SourceMapDto, StmtRefIr, UnitAddr,
};
use skiff_runtime_model::request_heap::RequestHeap;

use super::*;
use crate::{
    actor_executor_test_runtime as test_runtime,
    capabilities::{ExecutionControl, StreamRuntime},
    env::Env,
    heap_access::HeapAccess,
    program_execution::{EvaluatorControl, ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram, Interpreter,
};

#[derive(Clone)]
struct ProbeExecutionControl {
    cancelled: Arc<AtomicBool>,
    cancellation: CancellationToken,
    execution_scope: ExecutionScope,
}

impl Default for ProbeExecutionControl {
    fn default() -> Self {
        let cancellation = CancellationToken::new();
        Self {
            cancelled: cancellation.cancel_flag(),
            cancellation: cancellation.clone(),
            execution_scope: ExecutionScope::request(cancellation.clone(), None),
        }
    }
}

impl ExecutionControlApi for ProbeExecutionControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.execution_scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: skiff_artifact_model::InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        let execution_scope = self.execution_scope.derive(local_deadline, site)?;
        Ok(OwnedExecutionControl::new(ProbeExecutionControl {
            cancelled: Arc::clone(&self.cancelled),
            cancellation: self.cancellation.clone(),
            execution_scope,
        }))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.cancellation.is_cancelled() {
            Err(skiff_runtime_capability_context::ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        test_runtime::file_source_stream_context(stream_runtime)
    }
}

impl skiff_runtime_capability_context::OwnedExecutionControlApi for ProbeExecutionControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        &self.cancelled
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        None
    }
}

fn future_size<F: Future>(_future: F) -> usize {
    std::mem::size_of::<F>()
}

fn boxed_future_size<F: Future + ?Sized>(future: Pin<Box<F>>) -> usize {
    std::mem::size_of_val(&*future)
}

fn probe_context(
    control: ExecutionControl<'static>,
    stream_runtime: StreamRuntime,
) -> ProgramExecutionContext<'static> {
    let runtime_factory = test_runtime::runtime_factory();
    let test_effect_doubles = runtime_factory.reusable_test_effect_doubles(
        std::collections::HashMap::new(),
        &stream_runtime,
        false,
    );
    let effects = test_runtime::effects_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: control.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
        file: test_runtime::file_context(),
        file_source_stream: test_runtime::file_source_stream_context(stream_runtime.clone()),
        time: crate::capabilities::TimeCapabilityContext::new(control),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            crate::capabilities::HttpRuntimeOptions::explicit(false),
            stream_runtime,
            test_effect_doubles.clone(),
        ),
        test_effect_doubles,
        actor: test_runtime::actor_context(),
        request: test_runtime::request_context(),
        request_heap_limits: Default::default(),
    })
}

struct ProbeFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    addr: ExecutableAddr,
}

impl ProbeFixture {
    fn new() -> Self {
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:frame-diet-probe".to_string(),
            source_ast_hash: "source:frame-diet-probe".to_string(),
            module_path: "f445h.e4r.spine".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![LinkedExecutable {
                kind: ExecutableKind::Function,
                symbol: "probe".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: None,
                self_type: None,
                slots: SlotLayoutIr::default(),
                may_suspend: false,
                body: LinkedExecutableBody {
                    blocks: Vec::new(),
                    statements: Vec::new(),
                    expressions: Vec::new(),
                },
            }],
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/frame-diet-probe",
            vec![Arc::clone(&file)],
            Vec::new(),
            PublicationResourceTable::default(),
            Default::default(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        let interpreter = Interpreter::with_program(program, test_runtime::runtime_factory());
        let addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(file.file_ir_identity.clone()),
            executable: 0,
        };
        Self {
            interpreter,
            file,
            addr,
        }
    }

    fn eval_context<'a>(
        &'a self,
        context: ProgramExecutionContext<'static>,
        heap: &'a mut HeapAccess,
        env: &'a mut Env,
    ) -> EvalContext<'a> {
        EvalContext::new(
            &self.interpreter,
            context,
            heap,
            env,
            &self.addr,
            &self.file,
            &self.file.executables[0],
        )
        .expect("probe fixture eval context")
    }
}

#[test]
fn frame_diet_size_probe_reports_key_futures_and_contexts() {
    let fixture = ProbeFixture::new();
    let runtime_factory = test_runtime::runtime_factory();
    let stream_runtime = runtime_factory.stream_runtime();
    let control = ExecutionControl::new(ProbeExecutionControl::default());
    let context = probe_context(control, stream_runtime);
    let mut heap = HeapAccess::private(RequestHeap::default());
    let mut env =
        Env::for_program_executable(&fixture.file.executables[0], None, 0).expect("probe env");

    let block = BlockIr {
        label: "entry".to_string(),
        statements: vec![StmtRefIr { statement: 0 }],
    };
    let mut eval = fixture.eval_context(context.clone(), &mut heap, &mut env);
    let site = SyntheticInstructionSiteReason::CompilerGeneratedTestHarness;
    let call = CallIr {
        site: skiff_artifact_model::InstructionSourceSite::Synthetic { reason: site },
        target: LinkedCallTarget::Executable {
            addr: fixture.addr.clone(),
        },
        concrete_receiver: None,
        type_args: Default::default(),
        args: vec![ExprRefIr { expression: 0 }],
        inout_args: Vec::new(),
        metadata: Default::default(),
        actor_metadata: None,
    };
    let expr_call = LinkedExprIr::Call { call };
    let expr_literal = LinkedExprIr::Literal {
        value: LiteralIr::Null,
    };
    let statement = LinkedStmtIr::Return {
        value: Some(ExprRefIr { expression: 0 }),
    };

    eprintln!("=== frame-diet size probe (debug) ===");
    eprintln!(
        "ProgramExecutionContext<'static>       = {} bytes",
        std::mem::size_of::<ProgramExecutionContext<'static>>()
    );
    eprintln!(
        "Env                                   = {} bytes",
        std::mem::size_of::<Env>()
    );
    eprintln!(
        "EvaluatorControl                       = {} bytes",
        std::mem::size_of::<EvaluatorControl>()
    );
    eprintln!(
        "Flow                                   = {} bytes",
        std::mem::size_of::<crate::env::Flow>()
    );
    eprintln!(
        "HeapAccess                             = {} bytes",
        std::mem::size_of::<HeapAccess>()
    );
    eprintln!(
        "EvalContext instance                   = {} bytes",
        std::mem::size_of_val(&eval)
    );
    eprintln!(
        "eval_program_expr_ref (unboxed future) = {} bytes",
        future_size(eval.eval_program_expr_ref(ExprRefIr { expression: 0 }))
    );
    eprintln!(
        "eval_program_expr (boxed inner)        = {} bytes",
        boxed_future_size(eval.eval_program_expr(&expr_literal))
    );
    eprintln!(
        "eval_program_call (boxed inner)        = {} bytes",
        boxed_future_size(eval.eval_program_call(match &expr_call {
            LinkedExprIr::Call { call } => call,
            _ => unreachable!(),
        }))
    );
    eprintln!(
        "exec_program_statement_control (boxed inner) = {} bytes",
        boxed_future_size(eval.exec_program_statement_control(&statement))
    );
    eprintln!(
        "exec_program_block_body (unboxed)      = {} bytes",
        future_size(eval.exec_program_block_body(&block))
    );
    eprintln!(
        "exec_program_return (unboxed)          = {} bytes",
        future_size(eval.exec_program_return(Some(ExprRefIr { expression: 0 })))
    );
    eprintln!(
        "exec_program_block_control (unboxed)   = {} bytes",
        future_size(eval.exec_program_block_control("entry"))
    );
    drop(eval);
    eprintln!(
        "call_program_executable_carriers (boxed inner) = {} bytes",
        boxed_future_size(fixture.interpreter.call_program_executable_carriers(
            context.clone(),
            &mut heap,
            &env,
            &fixture.addr,
            &fixture.addr,
            &Default::default(),
            Vec::new(),
        ))
    );
    eprintln!(
        "exec_program_executable_ctx (boxed inner) = {} bytes",
        boxed_future_size(fixture.interpreter.exec_program_executable_ctx(
            context.clone(),
            &mut heap,
            &mut env,
            &fixture.addr,
            &fixture.file,
            &fixture.file.executables[0],
        ))
    );
}
