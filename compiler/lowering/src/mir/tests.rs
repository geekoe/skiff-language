//! MIR/CFG builder, liveness and `LoweredPackage` carriage tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use skiff_artifact_model::{
    CallTargetIr, CallableEffectSummary, ContractOperationId, ExecutableLinkTargetIr, ExprIr,
    ExprRefIr, InstructionSourceSite, PackageCallableId, ServiceCallRef, ServiceCallRefIndex,
    ServiceProtocolIdentity, StmtIr, TypeRefIr,
};
use skiff_compiler_input::CompilerPlatformSources;
use skiff_compiler_source::{
    build_package_from_parsed_sources_with_dependency_analysis,
    parsed_sources::parse_publication_sources, prelude_registry::initialize_prelude_registry,
    source_graph::CompilerSourceFile, CompileParsedPackageSourcesInput, PackageCompilePolicy,
    SourceDependencyAnalysisInput,
};

use crate::lower;
use crate::mir::builder::build_mir_units;
use crate::mir::liveness::compute_liveness;
use crate::mir::{
    MirBlock, MirBuildError, MirContractError, MirExecutableKind, MirExpression, MirForInBinding,
    MirForInItemKind, MirFunction, MirLiveness, MirParamMode, MirSlotKind, MirStmtKind,
    MirStreamResultFacts, MirUnit, MirWritablePlace, MirWritableRoot,
};

mod semantic_facts;

fn build_model(module_path: &str, source_text: &str) -> skiff_compiler_source::PackageSourceModel {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    initialize_prelude_registry(
        &CompilerPlatformSources::new(&platform_root).expect("workspace platform sources load"),
    )
    .expect("prelude registry initializes");
    let root = PathBuf::from("/mir-fixture");
    let relative = "internal/mir_fixture.skiff";
    let source = CompilerSourceFile::parse(
        PathBuf::from(relative),
        module_path.to_string(),
        false,
        false,
        source_text.to_string(),
        relative,
    )
    .expect("MIR fixture should parse");
    let production_sources = vec![source];
    let parsed_sources = parse_publication_sources(&root, &production_sources)
        .expect("MIR fixture source facts should build");
    build_package_from_parsed_sources_with_dependency_analysis(
        CompileParsedPackageSourcesInput {
            parsed_sources,
            production_sources,
            diagnostic_root: &root,
            publication_api: None,
            package_aliases: &BTreeMap::new(),
            package_dependencies: &[],
            package_facts: None,
            package_artifacts: None,
            policy: PackageCompilePolicy::new(PACKAGE_ID),
        },
        &SourceDependencyAnalysisInput::new([], []).unwrap(),
    )
    .expect("MIR fixture source model should build")
}

const MODULE: &str = "internal.mir_fixture";
const PACKAGE_ID: &str = "example.com/mir-fixture";

const MIR_FIXTURE: &str = r#"
  const answer: number = 42
  const backup: number = 7

  function answerCopy() -> number {
    return answer
  }

  function mirror(input: Array<number>) -> number {
    var acc = 0
    var i = 0
    while (i < input.length()) {
      if (acc == 0) {
        acc = 1
      }
      for item in input {
        match (item) {
          0 => { acc = acc + 1 }
          _ => { acc = acc + 2 }
        }
      }
      timeout(1ms) {
        acc = acc + 1
      }
      i = i + 1
    }
    return acc
  }

  type Problem { message: string }

  function catchy(input: Problem) -> CatchResult<Problem, Problem> {
    return catch<Problem>(input)
  }

  function pendingish() -> Stream<number> {
    emit 1
  }
"#;

fn mirror_function() -> MirFunction {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mir = &lowered.mir_units()[0];
    let function = mir
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.mirror"))
        .expect("mirror MirFunction");
    function.clone()
}

#[test]
fn mir_units_carried_by_lowered_package_with_effect_facts() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    assert_eq!(lowered.mir_units().len(), 1);
    let unit: &MirUnit = &lowered.mir_units()[0];
    assert_eq!(unit.module_path, MODULE);
    assert_eq!(
        unit.file_ir_identity,
        lowered.file_ir_units()[0].file_ir_identity
    );
    assert_eq!(
        unit.actor_declarations,
        lowered.file_ir_units()[0].actor_declarations
    );
    assert_eq!(unit.functions.len(), 4);
    unit.validate_executable_indices()
        .expect("executable indices are dense and unique");
    unit.validate_constants()
        .expect("constant indices are dense and unique");
    assert_eq!(unit.constants.len(), 2);
    let answer = unit.constant(0).expect("answer constant metadata");
    assert_eq!(answer.index, 0);
    assert_eq!(answer.symbol, format!("{MODULE}.answer"));
    assert_eq!(answer.ty, TypeRefIr::builtin("number"));
    assert_eq!(
        answer.source_span,
        lowered.file_ir_units()[0].declarations.constants["answer"].source_span
    );
    let answer_copy = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.answerCopy"))
        .expect("answerCopy");
    let load_const_index = answer_copy
        .expressions
        .iter()
        .find_map(|expression| match &expression.expression {
            ExprIr::LoadConst { const_index } => Some(*const_index),
            _ => None,
        })
        .expect("answerCopy loads a local constant");
    assert_eq!(unit.constant(load_const_index), Ok(answer));

    let mirror = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.mirror"))
        .expect("mirror");
    assert_eq!(mirror.kind, MirExecutableKind::Function);
    assert_eq!(
        mirror.executable_index,
        lowered.file_ir_units()[0].declarations.executables["mirror"].executable_index
    );
    assert_eq!(mirror.origin.file_ir_identity, unit.file_ir_identity);
    assert_eq!(mirror.origin.module_path, unit.module_path);
    assert_eq!(mirror.origin.executable_index, mirror.executable_index);
    assert_eq!(unit.function_by_origin(&mirror.origin), Ok(mirror));
    let mirror_position = unit
        .functions
        .iter()
        .position(|function| function.symbol == mirror.symbol)
        .and_then(|position| u32::try_from(position).ok())
        .expect("mirror MIR vector position fits u32");
    assert_ne!(
        mirror_position, mirror.executable_index,
        "declaration-name order must not be mistaken for executable-table order"
    );
    assert_eq!(
        unit.function_by_executable_index(mirror.executable_index),
        Ok(mirror)
    );
    assert_eq!(
        mirror.effect_summary_ref,
        PackageCallableId::new(format!(
            "pkg-callable:{PACKAGE_ID}:top-level:{MODULE}.mirror"
        )),
        "effect_summary_ref is the canonical typed implementation identity"
    );
    assert_eq!(
        &mirror.effect_summary,
        model
            .callable_effects()
            .operations()
            .get(&skiff_compiler_source::SourceSymbolKey::new(
                MODULE, "mirror"
            ))
            .expect("source mirror effect summary")
    );
    // A pure loop fixture has no pending effects.
    assert!(!mirror.may_pending());

    // Stream emit records PendingEffectCategory::Stream in the source effects.
    let pendingish = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.pendingish"))
        .expect("pendingish");
    assert!(
        pendingish.may_pending(),
        "stream fixture must be may_pending from source effects"
    );
    assert_eq!(
        pendingish.stream_result,
        Some(MirStreamResultFacts {
            item_type: TypeRefIr::builtin("number"),
        }),
        "Stream<T> item type survives into MirFunction"
    );
    assert_eq!(
        mirror.params[0],
        crate::mir::MirParam {
            name: "input".to_string(),
            slot: 0,
            ty: TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("number")],
            },
            mode: MirParamMode::Value,
        }
    );
    assert_eq!(mirror.slots[0].name, "input");
    assert_eq!(mirror.slots[0].kind, MirSlotKind::Param);
    assert!(!mirror.slots[0].writable_local);
    assert!(mirror.slots[0].ty.is_some());
    assert_eq!(mirror.slot_type(0), Ok(&mirror.params[0].ty));

    // MIR owns exact-index cloned expression/type pairs and its computed
    // liveness, while File IR retains the source facts used to construct it.
    let mirror_executable = lowered.file_ir_units()[0]
        .executables
        .iter()
        .find(|executable| executable.symbol == format!("{MODULE}.mirror"))
        .expect("mirror executable");
    assert_eq!(
        mirror_executable.expression_types.len(),
        mirror_executable.body.expressions.len(),
        "expression_types aligns with body.expressions"
    );
    assert_eq!(
        mirror.expressions.len(),
        mirror_executable.body.expressions.len()
    );
    for (index, expression) in mirror.expressions.iter().enumerate() {
        assert_eq!(expression.index as usize, index);
        assert_eq!(
            expression.expression,
            mirror_executable.body.expressions[index]
        );
        assert_eq!(expression.ty, mirror_executable.expression_types[index]);
        assert_eq!(
            mirror.expression(ExprRefIr {
                expression: expression.index,
            }),
            Ok(expression)
        );
    }
    assert_eq!(
        compute_liveness(mirror).expect("MIR-only liveness recomputation"),
        mirror.liveness
    );
    assert_eq!(
        mirror_executable.statement_spans.len(),
        mirror_executable.body.statements.len(),
        "statement_spans aligns with body.statements"
    );
    assert!(
        mirror_executable
            .statement_spans
            .iter()
            .any(Option::is_some),
        "source statements carry spans from the source facts"
    );
    assert!(mirror_executable
        .slots
        .slots
        .iter()
        .any(|slot| slot.ty.is_some()));

    let catchy = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.catchy"))
        .expect("catchy");
    // A pure string function has no pending effects.
    assert!(!catchy.may_pending());
}

#[test]
fn mir_remains_self_contained_after_original_file_ir_is_dropped() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut file_ir_units = lowered.file_ir_units().to_vec();
    let service_ref = ServiceCallRef {
        service_requirement_slot: 7,
        contract_operation_id: ContractOperationId::new("contract-operation:mirror"),
        expected_protocol_identity: ServiceProtocolIdentity::new("service-protocol:mirror"),
    };
    file_ir_units[0]
        .external_refs
        .service_call_refs
        .push(service_ref.clone());

    let mirror_executable_index =
        file_ir_units[0].declarations.executables["mirror"].executable_index;
    let mirror_index = mirror_executable_index as usize;
    file_ir_units[0].link_targets.executables.insert(
        "mirror".to_string(),
        ExecutableLinkTargetIr {
            executable_index: mirror_executable_index,
        },
    );
    let executable = &mut file_ir_units[0].executables[mirror_index];
    let service_expression_index = executable
        .body
        .expressions
        .iter()
        .position(|expression| matches!(expression, ExprIr::Call { .. }))
        .expect("mirror fixture has a call expression");
    let ExprIr::Call { call } = &mut executable.body.expressions[service_expression_index] else {
        unreachable!()
    };
    call.target = CallTargetIr::ServiceCall {
        service_call_ref_index: ServiceCallRefIndex::new(0),
    };
    let service_expression_index =
        u32::try_from(service_expression_index).expect("fixture expression index fits u32");
    let expected_expression =
        executable.body.expressions[service_expression_index as usize].clone();
    let expected_expression_type =
        executable.expression_types[service_expression_index as usize].clone();
    let expected_source_map = file_ir_units[0].source_map.clone();
    let expected_file_ir_identity = file_ir_units[0].file_ir_identity.clone();
    let expected_actor_declarations = file_ir_units[0].actor_declarations.clone();
    let expected_type_table = file_ir_units[0].type_table.clone();
    let expected_link_targets = file_ir_units[0].link_targets.clone();

    let mir_units = build_mir_units(PACKAGE_ID, &file_ir_units, model.callable_effects())
        .expect("self-contained MIR build");
    drop(file_ir_units);
    drop(lowered);
    drop(model);

    let unit = &mir_units[0];
    assert_eq!(unit.file_ir_identity, expected_file_ir_identity);
    assert_eq!(unit.actor_declarations, expected_actor_declarations);
    assert_eq!(
        unit.external_refs.service_call_refs,
        vec![service_ref.clone()]
    );
    assert_eq!(unit.source_map, expected_source_map);
    assert_eq!(unit.type_table, expected_type_table);
    assert_eq!(unit.link_targets, expected_link_targets);
    assert!(!unit.source_map.sources.is_empty());
    assert!(!unit.type_table.is_empty());
    assert!(unit.link_targets.executables.contains_key("mirror"));
    assert_eq!(
        unit.constant(0).expect("owned local constant").symbol,
        format!("{MODULE}.answer")
    );
    let mirror = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.mirror"))
        .expect("owned mirror MIR");
    assert_eq!(
        unit.function_by_executable_index(mirror.executable_index),
        Ok(mirror)
    );
    let expression = mirror
        .expression(ExprRefIr {
            expression: service_expression_index,
        })
        .expect("owned service-call expression");
    assert_eq!(expression.index, service_expression_index);
    assert_eq!(expression.expression, expected_expression);
    assert_eq!(expression.ty, expected_expression_type);
    let ExprIr::Call { call } = &expression.expression else {
        panic!("expected owned service-call expression")
    };
    let CallTargetIr::ServiceCall {
        service_call_ref_index,
    } = &call.target
    else {
        panic!("expected owned service-call target")
    };
    assert_eq!(
        unit.external_refs
            .service_call_refs
            .get((*service_call_ref_index).index() as usize),
        Some(&service_ref)
    );
    assert_eq!(
        compute_liveness(mirror).expect("liveness reads only owned MIR"),
        mirror.liveness
    );
    assert_eq!(mirror.liveness.blocks.len(), mirror.blocks.len());
}

#[test]
fn mir_builder_rejects_mismatched_expression_type_count() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut file_ir_units = lowered.file_ir_units().to_vec();
    let mirror_index =
        file_ir_units[0].declarations.executables["mirror"].executable_index as usize;
    let executable = &mut file_ir_units[0].executables[mirror_index];
    let expression_count = executable.body.expressions.len();
    executable
        .expression_types
        .pop()
        .expect("mirror fixture has expressions");
    let expression_type_count = executable.expression_types.len();

    let error = build_mir_units(PACKAGE_ID, &file_ir_units, model.callable_effects())
        .expect_err("mismatched expression types must fail closed");
    assert_eq!(
        error,
        MirBuildError::ExpressionTypeCountMismatch {
            module_path: MODULE.to_string(),
            symbol: format!("{MODULE}.mirror"),
            expression_count,
            expression_type_count,
        }
    );
}

#[test]
fn mir_builder_rejects_non_dense_or_duplicate_executable_indices() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut file_ir_units = lowered.file_ir_units().to_vec();
    let first_declaration = "answerCopy";
    let duplicate_declaration = "catchy";
    let executable_index =
        file_ir_units[0].declarations.executables[first_declaration].executable_index;
    file_ir_units[0]
        .declarations
        .executables
        .get_mut(duplicate_declaration)
        .expect("duplicate executable declaration")
        .executable_index = executable_index;

    let error = build_mir_units(PACKAGE_ID, &file_ir_units, model.callable_effects())
        .expect_err("duplicate executable indices must fail closed");
    assert_eq!(
        error,
        MirBuildError::DuplicateExecutableIndex {
            module_path: MODULE.to_string(),
            executable_index,
            first_declaration: first_declaration.to_string(),
            duplicate_declaration: duplicate_declaration.to_string(),
        }
    );

    let executable_index =
        u32::try_from(file_ir_units[0].executables.len()).expect("fixture count fits u32");
    file_ir_units[0]
        .declarations
        .executables
        .get_mut(duplicate_declaration)
        .expect("sparse executable declaration")
        .executable_index = executable_index;
    let error = build_mir_units(PACKAGE_ID, &file_ir_units, model.callable_effects())
        .expect_err("sparse executable indices must fail closed");
    assert_eq!(
        error,
        MirBuildError::MissingExecutable {
            module_path: MODULE.to_string(),
            declaration_name: duplicate_declaration.to_string(),
            executable_index,
        }
    );
}

#[test]
fn mir_builder_rejects_non_dense_or_duplicate_constant_indices() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut file_ir_units = lowered.file_ir_units().to_vec();
    let const_index = file_ir_units[0].declarations.constants["answer"].const_index;
    file_ir_units[0]
        .declarations
        .constants
        .get_mut("backup")
        .expect("backup constant declaration")
        .const_index = const_index;

    let error = build_mir_units(PACKAGE_ID, &file_ir_units, model.callable_effects())
        .expect_err("duplicate constant indices must fail closed");
    assert_eq!(
        error,
        MirBuildError::DuplicateConstantIndex {
            module_path: MODULE.to_string(),
            const_index,
            duplicate_declaration: "backup".to_string(),
        }
    );

    let const_index =
        u32::try_from(file_ir_units[0].constants.len()).expect("fixture count fits u32");
    file_ir_units[0]
        .declarations
        .constants
        .get_mut("backup")
        .expect("sparse constant declaration")
        .const_index = const_index;
    let error = build_mir_units(PACKAGE_ID, &file_ir_units, model.callable_effects())
        .expect_err("sparse constant indices must fail closed");
    assert_eq!(
        error,
        MirBuildError::ConstantIndexOutOfBounds {
            module_path: MODULE.to_string(),
            declaration_name: "backup".to_string(),
            const_index,
            constant_count: file_ir_units[0].constants.len(),
        }
    );
}

#[test]
fn mir_cfg_shapes_for_branches_loops_match_timeout_concurrent_and_catch() {
    let mirror = mirror_function();

    // Statement index -> MirStmt correspondence is recoverable: one entry per
    // MirStmt in flattened block order, carrying the File IR statement index.
    let flattened: Vec<u32> = mirror
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter().map(|stmt| stmt.statement_index))
        .collect();
    let entries: Vec<u32> = mirror
        .statements
        .iter()
        .map(|entry| entry.statement_index)
        .collect();
    assert_eq!(
        flattened, entries,
        "MirStmt stream matches statement entries"
    );

    // Entry block is id 0 and holds the loop + a Return continuation.
    let entry = &mirror.blocks[0];
    assert_eq!(entry.label, "entry");
    assert!(matches!(
        entry.statements.last().map(|stmt| &stmt.kind),
        Some(MirStmtKind::While { .. })
    ));
    assert_eq!(entry.successors.len(), 2, "while body + loop exit");

    // The while body's final fragment loops back to the While header.
    let while_body = mirror
        .blocks
        .iter()
        .find(|block| block.label.starts_with("while_body"))
        .expect("while body block");
    let last_while_fragment = mirror
        .blocks
        .iter()
        .filter(|block| block.label.starts_with("while_body"))
        .next_back()
        .expect("last while fragment");
    assert!(
        last_while_fragment.successors.contains(&entry.id),
        "loop-back edge to the While header"
    );
    assert!(
        entry.successors.contains(&while_body.id),
        "while body must be a successor of the entry block"
    );

    // ForIn inside the while body with its body block as successor.
    let for_in_block = mirror
        .blocks
        .iter()
        .find(|block| {
            block
                .statements
                .last()
                .is_some_and(|stmt| matches!(stmt.kind, MirStmtKind::ForIn { .. }))
        })
        .expect("for-in block");
    let MirStmtKind::ForIn {
        facts,
        body,
        continuation,
        ..
    } = &for_in_block
        .statements
        .last()
        .expect("for-in statement")
        .kind
    else {
        unreachable!()
    };
    assert_eq!(
        facts.iterable_type,
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        }
    );
    assert!(matches!(
        &facts.binding,
        MirForInBinding::Item {
            ty,
            kind: MirForInItemKind::ArrayItem,
            ..
        } if ty == &TypeRefIr::builtin("number")
    ));
    let mut expected_for_successors = vec![*body, *continuation];
    expected_for_successors.sort_unstable();
    assert_eq!(for_in_block.successors, expected_for_successors);
    assert!(mirror
        .block(*body)
        .expect("for body block")
        .label
        .starts_with("for_body"));
    mirror.block(*continuation).expect("for continuation block");

    // Match: arm bodies plus the no-match exit edge; the arm's completion
    // falls through to the next statement.
    let match_block = mirror
        .blocks
        .iter()
        .find(|block| {
            block
                .statements
                .last()
                .is_some_and(|stmt| matches!(stmt.kind, MirStmtKind::Match { .. }))
        })
        .expect("match block");
    assert_eq!(
        match_block.successors.len(),
        3,
        "two arm bodies + no-match exit"
    );
    assert!(matches!(
        &match_block.statements[0].kind,
        MirStmtKind::Match { arms, .. } if arms.len() == 2
    ));

    // Timeout statement references its body by block id.
    let timeout_block = mirror
        .blocks
        .iter()
        .find(|block| {
            block
                .statements
                .last()
                .is_some_and(|stmt| matches!(stmt.kind, MirStmtKind::Timeout { .. }))
        })
        .expect("timeout block");
    let MirStmtKind::Timeout {
        body,
        continuation,
        duration_ms,
        ..
    } = &timeout_block.statements[0].kind
    else {
        unreachable!()
    };
    assert_eq!(*duration_ms, 1);
    assert_eq!(timeout_block.successors, vec![*body]);

    // The timeout body completes back into the timeout statement's
    // continuation (the loop tail), so the body block is reachable.
    let timeout_body = mirror
        .blocks
        .iter()
        .find(|block| block.label.starts_with("timeout_body"))
        .expect("timeout body block");
    assert!(
        timeout_body.successors.contains(continuation),
        "timeout body must complete into the exact loop-tail continuation"
    );
    mirror
        .block(*continuation)
        .expect("timeout continuation block");

    // Catch expression produced exactly one exception region.
    let catchy_model = build_model(MODULE, MIR_FIXTURE);
    let catchy = lower(&catchy_model).expect("MIR fixture should lower");
    let catchy_unit = &catchy.file_ir_units()[0];
    let catchy_executable = catchy_unit
        .executables
        .iter()
        .find(|executable| executable.symbol == format!("{MODULE}.catchy"))
        .expect("catchy executable");
    let catchy_expr_index = catchy_executable
        .body
        .expressions
        .iter()
        .position(|expr| matches!(expr, ExprIr::Catch { .. }))
        .expect("catch expression");
    let catchy_mir = catchy.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.catchy"))
        .expect("catchy MirFunction");
    assert_eq!(catchy_mir.regions.len(), 1);
    let region = &catchy_mir.regions[0];
    if region.catch_expr as usize != catchy_expr_index {
        panic!(
            "region catch_expr {} != catch expr index {catchy_expr_index}; exprs: {:?}",
            region.catch_expr, catchy_executable.body.expressions
        );
    }
    assert_eq!(region.catch_type, TypeRefIr::LocalType { type_index: 0 });
    assert_eq!(region.cleanup_depth, 0);
}

#[test]
fn liveness_hand_computed_small_fixture() {
    // b0: let acc = const       -> def {1}, use {}
    // b1: acc = x               -> def {1}, use {0}
    // b2: return acc            -> def {}, use {1}
    // live_out(b2) = {}; live_in(b2) = {1}
    // live_out(b1) = live_in(b2) = {1}; live_in(b1) = {0} ∪ ({1} - {1}) = {0}
    // live_out(b0) = live_in(b1) = {0}; live_in(b0) = {} ∪ ({0} - {1}) = {0}
    let expressions = vec![
        MirExpression {
            index: 0,
            expression: ExprIr::LoadConst { const_index: 0 },
            ty: TypeRefIr::builtin("number"),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        },
        MirExpression {
            index: 1,
            expression: ExprIr::LoadSlot { slot: 0 },
            ty: TypeRefIr::builtin("number"),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        },
        MirExpression {
            index: 2,
            expression: ExprIr::LoadSlot { slot: 1 },
            ty: TypeRefIr::builtin("number"),
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        },
    ];
    let mut function = MirFunction {
        executable_index: 0,
        origin: skiff_artifact_model::PackageExecutableCoordinate {
            file_ir_identity: "file:m".to_string(),
            module_path: "m".to_string(),
            executable_index: 0,
        },
        symbol: "m.f".to_string(),
        kind: MirExecutableKind::Function,
        native: false,
        type_params: Vec::new(),
        params: vec![crate::mir::MirParam {
            name: "x".to_string(),
            slot: 0,
            ty: TypeRefIr::builtin("number"),
            mode: MirParamMode::Value,
        }],
        return_type: TypeRefIr::builtin("number"),
        self_type: None,
        receiver: None,
        slots: vec![
            crate::mir::MirSlot {
                slot: 0,
                name: "x".to_string(),
                kind: MirSlotKind::Param,
                writable_local: false,
                ty: None,
            },
            crate::mir::MirSlot {
                slot: 1,
                name: "acc".to_string(),
                kind: MirSlotKind::Local,
                writable_local: true,
                ty: None,
            },
        ],
        index_accesses: BTreeMap::new(),
        expression_blocks: BTreeMap::new(),
        expressions,
        blocks: vec![
            MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![crate::mir::MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::InitSlot {
                        slot: 1,
                        value: ExprRefIr { expression: 0 },
                    },
                }],
                successors: vec![1],
            },
            MirBlock {
                id: 1,
                label: "body".to_string(),
                statements: vec![crate::mir::MirStmt {
                    statement_index: 1,
                    span: None,
                    kind: MirStmtKind::Assign {
                        target: skiff_artifact_model::AssignTargetIr::Slot { slot: 1 },
                        place: MirWritablePlace {
                            root: MirWritableRoot::Slot { slot: 1 },
                            path: Vec::new(),
                        },
                        value: ExprRefIr { expression: 1 },
                    },
                }],
                successors: vec![2],
            },
            MirBlock {
                id: 2,
                label: "exit".to_string(),
                statements: vec![crate::mir::MirStmt {
                    statement_index: 2,
                    span: None,
                    kind: MirStmtKind::Return {
                        value: Some(ExprRefIr { expression: 2 }),
                    },
                }],
                successors: Vec::new(),
            },
        ],
        regions: Vec::new(),
        statements: Vec::new(),
        source_event_plan: crate::mir::MirSourceEventPlan::unavailable(
            crate::mir::MirSourceEventUnavailableReason::SourceFactsNotProvided,
        ),
        stream_result: None,
        liveness: MirLiveness::default(),
        effect_summary_ref: PackageCallableId::new("pkg-callable:test:top-level:m.f"),
        effect_summary: CallableEffectSummary::analysis_pending(),
        source_span: None,
    };

    let liveness = compute_liveness(&function).expect("MIR-only liveness");
    function.liveness = liveness.clone();
    assert_eq!(liveness.blocks[&0].live_in, vec![0]);
    assert_eq!(liveness.blocks[&0].live_out, vec![0]);
    assert_eq!(liveness.blocks[&1].live_in, vec![0]);
    assert_eq!(liveness.blocks[&1].live_out, vec![1]);
    assert_eq!(liveness.blocks[&2].live_in, vec![1]);
    assert_eq!(liveness.blocks[&2].live_out, Vec::<u32>::new());

    // Liveness is deterministic: recomputation yields identical output.
    let again = compute_liveness(&function).expect("MIR-only liveness recomputation");
    assert_eq!(again.blocks, liveness.blocks);
    assert_eq!(function.liveness, liveness);
    assert!(matches!(
        function.slot_type(0),
        Err(MirContractError::MissingSlotType { slot: 0, .. })
    ));
    assert!(function.validate_slot_types().is_err());
    let _: MirLiveness = liveness;
    let _: MirFunction = function;
    let _: InstructionSourceSite = InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerDesugaring,
    };
    let _: StmtIr = StmtIr::Break;
}

#[test]
fn value_block_expression_facts_freeze_ternary_and_user_value_blocks() {
    let model = build_model(
        MODULE,
        r#"
          function pick(flag: boolean) -> number {
            return flag ? 1 : 2
          }

          function blockish() -> number {
            return value {
              final local = 1
              local
            }
          }
        "#,
    );
    let lowered = lower(&model).expect("value block fixture should lower");
    let unit = &lowered.mir_units()[0];

    for declaration in ["pick", "blockish"] {
        let function = unit
            .functions
            .iter()
            .find(|function| function.symbol == format!("{MODULE}.{declaration}"))
            .unwrap_or_else(|| panic!("{declaration} MirFunction"));
        let facts = function
            .expressions
            .iter()
            .filter_map(|expression| match &expression.expression {
                ExprIr::ValueBlock { block, result } => {
                    Some((expression.index, block.clone(), *result))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(!facts.is_empty(), "{declaration} has a ValueBlock fact");
        for (index, block_label, result) in facts {
            let fact = function
                .expression_blocks
                .get(&index)
                .unwrap_or_else(|| panic!("{declaration} ValueBlock {index} has no fact"));
            assert_eq!(fact.result, result);
            assert_eq!(function.block(fact.body_block).expect("body block").label, block_label);
            assert!(!fact.completion_targets.is_empty());
            assert!(fact
                .completion_targets
                .windows(2)
                .all(|pair| pair[0] < pair[1]));
            for target in &fact.completion_targets {
                function.block(*target).expect("completion target block");
            }
        }
        function
            .validate_expression_block_facts()
            .expect("expression block facts remain contract-valid");
    }

    let pick = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.pick"))
        .expect("pick MirFunction");
    let pick_facts = pick
        .expression_blocks
        .values()
        .collect::<Vec<_>>();
    assert_eq!(pick_facts.len(), 1);
    let pick_body = pick.block(pick_facts[0].body_block).expect("ternary body block");
    assert!(matches!(
        pick_body.statements.last().map(|statement| &statement.kind),
        Some(MirStmtKind::If { .. })
    ));
    assert_eq!(pick_facts[0].completion_targets.len(), 1);
}

#[test]
fn config_intrinsics_route_to_native_call_targets() {
    let model = build_model(
        MODULE,
        r#"
          function configured() -> string {
            final required = config.require<string>("app.token")
            final optional = config.optional<string>("app.region")
            final present = config.has("app.enabled")
            return required
          }
        "#,
    );
    let lowered = lower(&model).expect("config fixture should lower");
    let function = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.configured"))
        .expect("configured MirFunction");
    let mut targets = function
        .expressions
        .iter()
        .filter_map(|expression| match &expression.expression {
            ExprIr::Call { call } => match &call.target {
                CallTargetIr::Native { target } => Some((
                    format!("{}.{}", target.namespace, target.symbol),
                    target.binding_key.clone(),
                )),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    targets.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        targets,
        vec![
            (
                "config.has".to_string(),
                Some("std.config.has".to_string())
            ),
            (
                "config.optional".to_string(),
                Some("std.config.optional".to_string())
            ),
            (
                "config.require".to_string(),
                Some("std.config.require".to_string())
            ),
        ]
    );
}

#[test]
fn concurrent_plan_lanes_become_block_ids_and_complete_into_continuation() {
    // A hand-built FileIrUnit exercising StmtIr::Concurrent (the source model
    // rejects `concurrent` in v1, so the CFG-ization is tested directly).
    use skiff_artifact_model::{
        BlockIr, ConcurrentLaneIr, ConcurrentPlanIr, ExecutableBody, ExecutableDeclarationIr,
        ExecutableIr, ExecutableKind, FileDeclarations, FileIrUnit, FileLinkTargets, SlotLayout,
        SourceMapDto, StmtRefIr, SyntheticInstructionSiteReason,
    };
    let body = ExecutableBody {
        blocks: vec![
            BlockIr {
                label: "concurrent_lane$0".to_string(),
                statements: Vec::new(),
            },
            BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            },
        ],
        statements: vec![
            StmtIr::Concurrent {
                plan: ConcurrentPlanIr {
                    lanes: vec![ConcurrentLaneIr::Statement {
                        source_order: 0,
                        dependencies: Vec::new(),
                        body: "concurrent_lane$0".to_string(),
                        site: InstructionSourceSite::Synthetic {
                            reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                        },
                    }],
                    site: InstructionSourceSite::Synthetic {
                        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                    },
                },
            },
            StmtIr::Return { value: None },
        ],
        expressions: Vec::new(),
    };
    let executable = ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "m.concurrentish".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body,
        expression_types: Vec::new(),
        statement_spans: vec![None, None],
        source_span: None,
    };
    let unit = FileIrUnit {
        schema_version: "1".to_string(),
        file_ir_identity: "file:m".to_string(),
        source_ast_hash: String::new(),
        module_path: "m".to_string(),
        ir_format_version: "1".to_string(),
        opcode_table_version: "1".to_string(),
        required_receiver_builtin_capability_version: 0,
        source_map: SourceMapDto {
            format: "skiff-file-ir-source-map-v1".to_string(),
            sources: Vec::new(),
            spans: Vec::new(),
        },
        actor_declarations: Vec::new(),
        declarations: FileDeclarations {
            types: BTreeMap::new(),
            interfaces: BTreeMap::new(),
            db: BTreeMap::new(),
            executables: BTreeMap::from([(
                "concurrentish".to_string(),
                ExecutableDeclarationIr {
                    executable_index: 0,
                    symbol: "m.concurrentish".to_string(),
                    source_span: None,
                },
            )]),
            constants: BTreeMap::new(),
        },
        link_targets: FileLinkTargets::default(),
        type_table: Vec::new(),
        constants: Vec::new(),
        executables: vec![executable],
        external_refs: Default::default(),
    };
    let per_callable = BTreeMap::from([(
        skiff_compiler_source::SourceSymbolKey::new("m", "concurrentish"),
        CallableEffectSummary::analysis_pending(),
    )]);
    let mir = crate::mir::builder::build_mir_unit_with_effect_map(
        "example.com/concurrent-fixture",
        &unit,
        &per_callable,
    )
    .expect("concurrent fixture should build MIR");
    let function = &mir.functions[0];
    assert_eq!(
        function.effect_summary_ref,
        PackageCallableId::new(
            "pkg-callable:example.com/concurrent-fixture:top-level:m.concurrentish"
        )
    );
    assert_eq!(
        function.effect_summary,
        CallableEffectSummary::analysis_pending()
    );
    assert!(
        function.may_pending(),
        "unknown effect analysis is conservatively pending"
    );

    // entry: [Concurrent] then continuation [Return].
    let entry = &function.blocks[0];
    let MirStmtKind::Concurrent { plan } = &entry.statements[0].kind else {
        panic!("expected concurrent statement")
    };
    let expected_site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
    };
    assert_eq!(plan.site, expected_site);
    assert_eq!(plan.lanes.len(), 1);
    let crate::mir::MirConcurrentLaneIr::Statement { body, site, .. } = &plan.lanes[0] else {
        panic!("expected statement lane")
    };
    assert_eq!(site, &expected_site);
    let lane_id = *body;
    assert!(function.blocks.iter().any(|block| block.id == lane_id));
    assert_eq!(entry.successors, vec![lane_id]);

    // The lane's Return terminates; the statement continuation (entry's next
    // fragment) holds the Return and has no successors.
    let lane_block = function
        .blocks
        .iter()
        .find(|block| block.id == lane_id)
        .expect("lane block");
    assert!(lane_block.statements.is_empty());
    assert_eq!(lane_block.successors, vec![plan.join_block]);
    let join_block = function
        .block(plan.join_block)
        .expect("checked concurrent join block");
    assert!(matches!(
        join_block.statements.last().map(|stmt| &stmt.kind),
        Some(MirStmtKind::Return { .. })
    ));
    assert!(join_block.successors.is_empty());
    assert_eq!(function.statements.len(), 2);

    let mut malformed = unit.clone();
    let StmtIr::Concurrent { plan } = &mut malformed.executables[0].body.statements[0] else {
        unreachable!()
    };
    let ConcurrentLaneIr::Statement { source_order, .. } = &mut plan.lanes[0] else {
        unreachable!()
    };
    *source_order = 1;
    let error = crate::mir::builder::build_mir_unit_with_effect_map(
        "example.com/concurrent-fixture",
        &malformed,
        &per_callable,
    )
    .expect_err("non-canonical concurrent order must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("stores source_order 1")
    ));
}

#[test]
fn phase_3_union_catch_fixture_lowers_with_union_bindings_and_aligned_rethrow() {
    let model = build_model(
        MODULE,
        r#"
  type LeafA {
    marker: number,
    owner: Array<number>,
  }

  type LeafB {
    marker: number,
    owner: Array<number>,
  }

  function innerThrow(leaf: LeafA | LeafB) -> void {
    final cleanupOwner = [7]
    throw leaf
  }

  function run(seed: number) -> number {
    if seed == 1 {
      final leaf: LeafA | LeafB = LeafA { marker: seed, owner: [seed] }
      final inner = catch<LeafA>(innerThrow(leaf))
      if inner.tag == "err" {
        final exc = inner.exception
        final outer = catch<LeafA>(rethrow exc)
        if outer.tag == "err" {
          return 2
        }
        return 11
      }
      return 12
    }
    final leaf: LeafA | LeafB = LeafB { marker: seed, owner: [seed] }
    final caught = catch<LeafB>(innerThrow(leaf))
    if caught.tag == "err" {
      return 3
    }
    return 13
  }
"#,
    );
    let lowered = lower(&model).expect("phase 3 union catch fixture should lower");
    let unit = &lowered.mir_units()[0];
    let run = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run");

    // Both `leaf` bindings carry the declared anonymous union, not the
    // constructor branch: the constructor enters the union context.
    let leaf_slots = run
        .slots
        .iter()
        .filter(|slot| slot.name == "leaf")
        .collect::<Vec<_>>();
    assert_eq!(leaf_slots.len(), 2);
    for leaf in leaf_slots {
        assert!(matches!(
            leaf.ty,
            Some(TypeRefIr::Union { .. })
        ), "leaf binding should widen into the declared union, got {:?}", leaf.ty);
    }

    // The caught payload binding carries the opaque Exception<LeafA>
    // envelope and the catch-over-rethrow result is CatchResult<never, LeafA>.
    let exc = run
        .slots
        .iter()
        .find(|slot| slot.name == "exc")
        .expect("exc binding");
    assert_eq!(
        exc.ty.as_ref(),
        Some(&TypeRefIr::Builtin {
            name: "Exception".to_string(),
            args: vec![TypeRefIr::LocalType { type_index: 0 }],
        })
    );
    let outer = run
        .slots
        .iter()
        .find(|slot| slot.name == "outer")
        .expect("outer binding");
    assert_eq!(
        outer.ty.as_ref(),
        Some(&TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![
                TypeRefIr::builtin("never"),
                TypeRefIr::LocalType { type_index: 0 },
            ],
        })
    );

    // The expression-form rethrow consumed the exception identifier's source
    // ExpressionKey, so the trailing LeafB constructor fact stays aligned
    // (this fixture only lowers when the rethrow key bookkeeping is exact).
    let rethrow_slot = run
        .slots
        .iter()
        .find(|slot| slot.name == "exc")
        .expect("exc binding")
        .slot;
    assert!(
        run.expressions.iter().any(|expression| {
            matches!(
                expression.expression,
                ExprIr::Rethrow { exception_slot } if exception_slot == rethrow_slot
            )
        }),
        "expression-form rethrow should reference the exc slot"
    );

    // `throw leaf` in innerThrow carries the union payload type so union
    // actual-leaf identity stays runtime-captured.
    let inner_throw = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.innerThrow"))
        .expect("innerThrow");
    assert!(inner_throw.blocks.iter().flat_map(|block| &block.statements).any(
        |statement| matches!(
            &statement.kind,
            MirStmtKind::Throw { payload_type, .. }
                if matches!(payload_type, TypeRefIr::Union { .. })
        )
    ));
}

#[test]
fn phase_3_catch_result_slot_gets_a_static_type() {
    let model = build_model(
        MODULE,
        r#"
  type LeafA {
    marker: number,
  }

  type LeafB {
    marker: number,
  }

  function throwLeaf(leaf: LeafA | LeafB) -> void {
    throw leaf
  }

  function run(seed: number) -> number {
    final attempt = catch<LeafB>(throwLeaf(LeafA { marker: seed }))
    if attempt.tag == "ok" {
      return 7
    }
    return 99
  }
"#,
    );
    let lowered = lower(&model).expect("phase 3 mismatch catch fixture should lower");
    let unit = &lowered.mir_units()[0];
    let run = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run");
    let attempt = run
        .slots
        .iter()
        .find(|slot| slot.name == "attempt")
        .expect("attempt binding");
    assert_eq!(
        attempt.ty.as_ref(),
        Some(&TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![
                TypeRefIr::builtin("void"),
                TypeRefIr::LocalType { type_index: 1 },
            ],
        }),
        "catch result bindings must receive a concrete slot type"
    );
}

#[test]
fn phase_3_catch_over_throw_binding_gets_a_never_result_slot_type() {
    let model = build_model(
        MODULE,
        r#"
  type LeafA {
    marker: number,
  }

  type LeafB {
    marker: number,
  }

  function run(seed: number) -> number {
    final attempt = catch<LeafB>(throw LeafA { marker: seed })
    if attempt.tag == "ok" {
      return 7
    }
    return 99
  }
"#,
    );
    let lowered = lower(&model).expect("converged mismatch fixture should lower");
    let unit = &lowered.mir_units()[0];
    let run = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run");
    let attempt = run
        .slots
        .iter()
        .find(|slot| slot.name == "attempt")
        .expect("attempt binding");
    assert_eq!(
        attempt.ty.as_ref(),
        Some(&TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![
                TypeRefIr::builtin("never"),
                TypeRefIr::LocalType { type_index: 1 },
            ],
        }),
        "catch over a throw expression should type its slot CatchResult<never, E>"
    );
}

#[test]
fn phase_3_rethrow_with_non_identifier_operand_fails_closed() {
    let model = build_model(
        MODULE,
        r#"
  type LeafA {
    marker: number,
  }

  function run() -> void {
    final inner = catch<LeafA>(throw LeafA { marker: 1 })
    final e = rethrow inner.exception
  }
"#,
    );
    let error = lower(&model).expect_err("field-operand rethrow must fail closed");
    match error {
        skiff_compiler_source::SourceCompileError::ContractValidation { message } => {
            assert!(
                message
                    .contains("rethrow in typed File IR requires an exception slot identifier"),
                "unexpected rethrow diagnostic: {message}"
            );
        }
        other => panic!("expected contract validation failure, got {other:?}"),
    }
}
