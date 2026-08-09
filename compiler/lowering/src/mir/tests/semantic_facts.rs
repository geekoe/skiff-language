use super::*;

use skiff_artifact_model::{InOutPathSegmentIr, PatternIr};

const WRITABLE_FIXTURE: &str = r#"
  type Inner { values: Array<number>, count: number }
  type Outer { inner: Inner }

  function inc(prefix: string, inout value: number, suffix: string) -> void {
    value = value + 1
  }

  function run(source: Array<number>) -> number {
    var outer = Outer { inner: Inner { values: source, count: 0 } }
    outer.inner.count = 2
    outer.inner.values.push(3)
    outer.inner.values[0] = 4
    let first = outer.inner.values[0]
    inc("before", inout outer.inner.count, "after")
    inc("before", inout outer.inner.values[0], "after")
    return outer.inner.count + first
  }
"#;

const WRITABLE_NO_INDEX_FIXTURE: &str = r#"
  type Inner { values: Array<number>, count: number }
  type Outer { inner: Inner }

  function inc(prefix: string, inout value: number, suffix: string) -> void {
    value = value + 1
  }

  function run(source: Array<number>) -> number {
    var outer = Outer { inner: Inner { values: source, count: 0 } }
    inc("before", inout outer.inner.count, "after")
    return outer.inner.count
  }
"#;

const RECORD_PATTERN_FIXTURE: &str = r#"
  function run(payload: { kind: string, body: { state: string } }) -> string {
    match payload {
      { kind: "ok", body: { state } } => {
        return state
      }
      _ => {
        return "other"
      }
    }
  }
"#;

const RECEIVER_FIXTURE: &str = r#"
  type Counter { value: number }

  impl Counter {
    function implicit(delta: number) -> number {
      return self.value + delta
    }

    function explicit(self: Counter, delta: number) -> number {
      return self.value + delta
    }
  }

  function run(counter: Counter) -> number {
    return counter.implicit(1) + counter.explicit(2)
  }
"#;

#[test]
fn implicit_and_explicit_receivers_share_one_exact_mir_contract() {
    let model = build_model(MODULE, RECEIVER_FIXTURE);
    let lowered = lower(&model).expect("receiver fixture should lower");
    let unit = &lowered.mir_units()[0];
    let implicit = unit
        .functions
        .iter()
        .find(|function| function.symbol.ends_with("Counter.implicit"))
        .expect("implicit receiver MIR");
    let explicit = unit
        .functions
        .iter()
        .find(|function| function.symbol.ends_with("Counter.explicit"))
        .expect("explicit receiver MIR");
    for function in [implicit, explicit] {
        function
            .validate_receiver_facts()
            .expect("checked unified receiver");
        assert_eq!(function.receiver.as_ref().expect("receiver").slot, 0);
        assert_eq!(
            function
                .receiver
                .as_ref()
                .expect("receiver")
                .parameter_ordinal,
            0
        );
        assert_eq!(
            function.receiver.as_ref().expect("receiver").call_abi,
            skiff_artifact_model::ReceiverCallAbi::ExplicitSelfFirst
        );
        assert_eq!(
            function.receiver.as_ref().expect("receiver").ty,
            function.self_type.clone().expect("required self type")
        );
    }
    assert_eq!(implicit.slots[0].kind, MirSlotKind::SelfValue);
    assert_eq!(implicit.params[0].slot, 1);
    assert_eq!(explicit.slots[0].kind, MirSlotKind::Param);
    assert_eq!(explicit.params[0].name, "self");
    assert_eq!(explicit.params[0].slot, 0);
    assert_eq!(explicit.params[0].mode, MirParamMode::Value);

    let run = unit
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run MIR");
    let receiver_calls = run
        .expressions
        .iter()
        .filter_map(|expression| expression.direct_call.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(receiver_calls.len(), 2);
    for call in receiver_calls {
        assert!(call.concrete_receiver.is_some());
        assert_eq!(
            call.receiver_call_abi,
            Some(skiff_artifact_model::ReceiverCallAbi::ExplicitSelfFirst)
        );
        assert_eq!(call.parameter_modes[0], MirParamMode::Value);
        assert!(matches!(
            call.argument(0),
            Some(crate::mir::MirCallArgument::Value { .. })
        ));
    }
}

#[test]
fn publication_and_package_direct_calls_retain_required_concrete_receivers() {
    let model = build_model(MODULE, RECEIVER_FIXTURE);
    let lowered = lower(&model).expect("receiver fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let run_index = units[0].declarations.executables["run"].executable_index as usize;
    let package_callable_id = skiff_artifact_model::PackageCallableId::new(
        "pkg-callable:example.com/dependency:top-level:Counter.explicit",
    );
    {
        let mut calls = units[0].executables[run_index]
            .body
            .expressions
            .iter_mut()
            .filter_map(|expression| match expression {
                ExprIr::Call { call } if call.concrete_receiver.is_some() => Some(call),
                _ => None,
            });
        let publication = calls.next().expect("publication receiver call");
        let executable_index = match &publication.target {
            CallTargetIr::LocalExecutable { executable_index } => *executable_index,
            other => panic!("expected local receiver target, found {other:?}"),
        };
        publication.target = CallTargetIr::PublicationExecutable {
            module_path: MODULE.to_string(),
            executable_index,
        };
        let package = calls.next().expect("package receiver call");
        package.target = CallTargetIr::PackageCallable {
            package_ref: skiff_artifact_model::PackageRefIr::Dependency {
                dependency_ref: "dependency".to_string(),
            },
            package_callable_id: package_callable_id.clone(),
        };
    }

    let targets = skiff_compiler_source::ResolvedCallTargetFacts::from_targets(BTreeMap::from([(
        skiff_compiler_source::ExpressionKey::new(
            MODULE,
            skiff_compiler_source::ExpressionOwnerKey::Function("run".to_string()),
            0,
        ),
        skiff_compiler_source::ResolvedCallTarget::DependencyPackageFunction {
            package_requirement_alias: "dependency".to_string(),
            compiler_owned: false,
            package_callable_id: package_callable_id.clone(),
            expected_local_abi: skiff_artifact_model::PackageLocalAbiIdentity::new(
                "local-abi:dependency",
            ),
            exact_signature: Some(skiff_artifact_model::PackageCallableSignature {
                type_params: Vec::new(),
                parameters: vec![
                    skiff_artifact_model::PackageCallableParameter {
                        name: "self".to_string(),
                        ty: skiff_artifact_model::PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("Counter"),
                        },
                        mode: skiff_artifact_model::ParamModeIr::Value,
                    },
                    skiff_artifact_model::PackageCallableParameter {
                        name: "delta".to_string(),
                        ty: skiff_artifact_model::PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("number"),
                        },
                        mode: skiff_artifact_model::ParamModeIr::Value,
                    },
                ],
                return_type: skiff_artifact_model::PackageTypeRef::Local {
                    local_type: TypeRefIr::builtin("number"),
                },
                may_suspend: false,
            }),
            inout_parameters: BTreeMap::new(),
        },
    )]));
    let mir = crate::mir::builder::build_mir_units_with_call_facts(
        PACKAGE_ID,
        &units,
        model.callable_effects(),
        &targets,
    )
    .expect("publication/package receiver facts should build");
    let run = mir[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run MIR");
    let direct = run
        .expressions
        .iter()
        .filter_map(|expression| expression.direct_call.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(direct.len(), 2);
    assert!(direct.iter().all(|call| call.concrete_receiver.is_some()));
    assert!(direct.iter().all(|call| {
        call.receiver_call_abi == Some(skiff_artifact_model::ReceiverCallAbi::ExplicitSelfFirst)
            && call.parameter_modes.first() == Some(&MirParamMode::Value)
    }));
}

#[test]
fn receiver_bound_direct_call_without_concrete_receiver_is_rejected() {
    let model = build_model(MODULE, RECEIVER_FIXTURE);
    let lowered = lower(&model).expect("receiver fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let run_index = units[0].declarations.executables["run"].executable_index as usize;
    let call = units[0].executables[run_index]
        .body
        .expressions
        .iter_mut()
        .find_map(|expression| match expression {
            ExprIr::Call { call } if call.concrete_receiver.is_some() => Some(call),
            _ => None,
        })
        .expect("receiver call");
    call.concrete_receiver = None;

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("receiver-bound direct call must carry concreteReceiver");
    assert!(matches!(
        error,
        MirBuildError::InvalidDirectCallFacts { ref message, .. }
            if message.contains("receiver requirement disagrees")
    ));
}

#[test]
fn actor_declaration_authority_remains_unit_owned_after_file_ir_drop() {
    let model = build_model(
        MODULE,
        r#"
          type UserActor { id: string, name: string }
          actor UserActor {
            key(id)
            create(name: string)
          }
          impl UserActor {
            function create(self: UserActor, name: string) -> void {
              self.name = name
            }
            function rename(self: UserActor, name: string) -> string {
              self.name = name
              return self.name
            }
          }
        "#,
    );
    let lowered = lower(&model).expect("actor fixture should lower");
    let expected = lowered.file_ir_units()[0].actor_declarations.clone();
    let unit = lowered.mir_units()[0].clone();
    drop(lowered);
    drop(model);

    assert_eq!(unit.actor_declarations, expected);
    let actor = unit.actor_declarations.first().expect("owned actor row");
    for executable_index in actor.method_implementations.values().copied().chain(
        actor
            .create_implementation
            .iter()
            .map(|create| create.executable_index),
    ) {
        assert!(unit.function_by_executable_index(executable_index).is_ok());
    }
}

#[test]
fn writable_places_and_inout_loans_remain_owned_after_file_ir_drop() {
    let model = build_model(MODULE, WRITABLE_FIXTURE);
    let lowered = lower(&model).expect("writable fixture should lower");
    let function = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run MIR")
        .clone();
    drop(lowered);
    drop(model);

    assert_eq!(function.index_accesses.len(), 3);
    function
        .validate_index_accesses()
        .expect("index facts remain self-contained");

    let outer_slot = function
        .slots
        .iter()
        .find(|slot| slot.name == "outer")
        .expect("outer slot")
        .slot;
    assert!(
        function
            .slot(outer_slot)
            .expect("outer slot")
            .writable_local
    );
    assert!(function
        .writable_local_slots()
        .expect("checked writable locals")
        .contains(&outer_slot));
    let assignment_place = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            MirStmtKind::Assign { place, .. }
                if place.path
                    == vec![
                        crate::mir::MirWritablePathSegment::Field {
                            name: "inner".to_string(),
                        },
                        crate::mir::MirWritablePathSegment::Field {
                            name: "count".to_string(),
                        },
                    ] =>
            {
                Some(place)
            }
            _ => None,
        })
        .expect("nested assignment place");
    assert_eq!(
        assignment_place.root,
        MirWritableRoot::Slot { slot: outer_slot }
    );

    let (mutating_expression, mutating_place) = function
        .expressions
        .iter()
        .find_map(|expression| {
            expression
                .writable
                .as_ref()
                .and_then(|facts| facts.mutating_receiver.as_ref())
                .map(|place| (expression, place))
        })
        .expect("mutating receiver place");
    let ExprIr::Call { call } = &mutating_expression.expression else {
        unreachable!()
    };
    assert!(matches!(&call.site, InstructionSourceSite::Source { .. }));
    assert_eq!(
        mutating_place,
        &MirWritablePlace {
            root: MirWritableRoot::Slot { slot: outer_slot },
            path: vec![
                crate::mir::MirWritablePathSegment::Field {
                    name: "inner".to_string(),
                },
                crate::mir::MirWritablePathSegment::Field {
                    name: "values".to_string(),
                },
            ],
        }
    );
    let checked_mutating = function
        .call_writable_facts(ExprRefIr {
            expression: mutating_expression.index,
        })
        .expect("checked mutating facts")
        .expect("stored mutating facts");
    assert_eq!(
        checked_mutating,
        mutating_expression.writable.as_ref().unwrap()
    );

    let (loan_expression, loan) = function
        .expressions
        .iter()
        .find_map(|expression| {
            expression
                .writable
                .as_ref()
                .and_then(|facts| facts.inout_loans.first())
                .map(|loan| (expression, loan))
        })
        .expect("inout loan");
    assert_eq!(loan.parameter_ordinal, 1);
    assert_eq!(loan.root_slot, outer_slot);
    assert_eq!(
        loan.path,
        vec![
            crate::mir::MirInOutPathSegment::Field {
                name: "inner".to_string(),
            },
            crate::mir::MirInOutPathSegment::Field {
                name: "count".to_string(),
            },
        ]
    );
    let checked_loan = function
        .call_writable_facts(ExprRefIr {
            expression: loan_expression.index,
        })
        .expect("checked inout facts")
        .expect("stored inout facts");
    assert_eq!(checked_loan, loan_expression.writable.as_ref().unwrap());
    let direct = function
        .direct_call_facts(ExprRefIr {
            expression: loan_expression.index,
        })
        .expect("checked direct call")
        .expect("direct call facts");
    assert_eq!(
        direct.parameter_modes,
        vec![
            MirParamMode::Value,
            MirParamMode::InOut,
            MirParamMode::Value
        ]
    );
    assert!(matches!(
        direct.argument(0),
        Some(crate::mir::MirCallArgument::Value { .. })
    ));
    assert!(matches!(
        direct.argument(1),
        Some(crate::mir::MirCallArgument::InOut { loan })
            if loan.parameter_ordinal == 1
    ));
    assert!(matches!(
        direct.argument(2),
        Some(crate::mir::MirCallArgument::Value { .. })
    ));
}

#[test]
fn inout_index_selector_and_type_are_owned_and_missing_selector_is_rejected() {
    let model = build_model(MODULE, WRITABLE_FIXTURE);
    let lowered = lower(&model).expect("writable fixture should lower");
    let run = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run MIR");
    let policies = run
        .index_accesses
        .values()
        .map(|access| access.policy)
        .collect::<BTreeSet<_>>();
    assert_eq!(run.index_accesses.len(), 3);
    assert_eq!(
        policies,
        BTreeSet::from([
            crate::mir::MirIndexPolicy::StrictRead,
            crate::mir::MirIndexPolicy::TerminalReplace,
            crate::mir::MirIndexPolicy::LoanMustExist,
        ])
    );
    assert!(run.index_accesses.values().all(|access| {
        access.receiver_kind == crate::mir::MirIndexReceiverKind::Array
            && access.receiver_type
                == TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::builtin("number")],
                }
    }));
    let segment = run
        .expressions
        .iter()
        .filter_map(|expression| expression.writable.as_ref())
        .flat_map(|facts| &facts.inout_loans)
        .flat_map(|loan| &loan.path)
        .find_map(|segment| match segment {
            crate::mir::MirInOutPathSegment::Index {
                selector,
                selector_type,
                access,
            } => Some((*selector, selector_type, access)),
            crate::mir::MirInOutPathSegment::Field { .. } => None,
        })
        .expect("typed selector segment");
    assert_eq!(segment.1, &TypeRefIr::builtin("integer"));
    assert_eq!(&segment.2.selector_type, segment.1);
    assert_eq!(segment.2.result_type, TypeRefIr::builtin("number"));
    assert_eq!(segment.2.policy, crate::mir::MirIndexPolicy::LoanMustExist);
    assert_eq!(
        run.index_access(segment.0).expect("checked source fact"),
        segment.2
    );

    let mut missing_fact = run.clone();
    missing_fact.index_accesses.remove(&segment.0.expression);
    assert!(matches!(
        missing_fact.validate_index_accesses(),
        Err(crate::mir::MirContractError::MissingIndexAccessFacts {
            selector,
            ..
        }) if selector == segment.0.expression
    ));

    let mut corrupt = run.clone();
    let (expression_index, call) = corrupt
        .expressions
        .iter_mut()
        .find_map(|expression| match &mut expression.expression {
            ExprIr::Call { call }
                if call.inout_args.iter().any(|loan| {
                    loan.path
                        .iter()
                        .any(|segment| matches!(segment, InOutPathSegmentIr::Index { .. }))
                }) =>
            {
                Some((expression.index, call))
            }
            _ => None,
        })
        .expect("indexed inout call");
    let index_segment = call
        .inout_args
        .iter_mut()
        .flat_map(|loan| &mut loan.path)
        .find(|segment| matches!(segment, InOutPathSegmentIr::Index { .. }))
        .expect("raw index segment");
    *index_segment = InOutPathSegmentIr::Index {
        selector: ExprRefIr {
            expression: u32::MAX,
        },
    };
    let error = corrupt
        .call_writable_facts(ExprRefIr {
            expression: expression_index,
        })
        .expect_err("missing inout selector expression must fail closed");
    assert!(matches!(
        error,
        crate::mir::MirContractError::InvalidWritableFacts {
            ref message,
            ..
        } if message.contains("missing expression")
    ));
}

#[test]
fn inout_root_without_source_writable_local_fact_is_rejected() {
    let model = build_model(MODULE, WRITABLE_NO_INDEX_FIXTURE);
    let lowered = lower(&model).expect("writable fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let run_index = units[0].declarations.executables["run"].executable_index as usize;
    let root_slot = units[0].executables[run_index]
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            ExprIr::Call { call } => call.inout_args.first().map(|loan| loan.root_slot),
            _ => None,
        })
        .expect("inout root slot");
    units[0].executables[run_index].slots.slots[root_slot as usize].writable_local = false;

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("loan root without writableLocal must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidWritableFacts { ref message, .. }
            if message.contains("source-confirmed writable local")
    ));
}

#[test]
fn inout_parameter_ordinal_must_match_exact_callee_modes() {
    let model = build_model(MODULE, WRITABLE_NO_INDEX_FIXTURE);
    let lowered = lower(&model).expect("writable fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let run_index = units[0].declarations.executables["run"].executable_index as usize;
    let loan = units[0].executables[run_index]
        .body
        .expressions
        .iter_mut()
        .find_map(|expression| match expression {
            ExprIr::Call { call } => call.inout_args.first_mut(),
            _ => None,
        })
        .expect("inout loan");
    loan.parameter_ordinal = 0;

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("loan ordinal at a Value parameter must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidDirectCallFacts { ref message, .. }
            if message.contains("missing inout loan for parameter 1")
                || message.contains("both Value and inout")
    ));
}

#[test]
fn nested_record_patterns_are_retained_and_validated_recursively() {
    let model = build_model(MODULE, RECORD_PATTERN_FIXTURE);
    let lowered = lower(&model).expect("record-pattern fixture should lower");
    let function = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.run"))
        .expect("run MIR")
        .clone();
    drop(lowered);
    drop(model);

    let pattern = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            MirStmtKind::Match { arms, .. } => arms.first().map(|arm| &arm.pattern),
            _ => None,
        })
        .expect("record match arm");
    let PatternIr::Record { fields } = pattern else {
        panic!("outer record pattern")
    };
    let body = fields
        .iter()
        .find(|field| field.name == "body")
        .expect("body field");
    let PatternIr::Record { fields } = &body.pattern else {
        panic!("nested body record pattern")
    };
    let PatternIr::Binding { slot } = &fields[0].pattern else {
        panic!("nested state binding")
    };
    assert_eq!(
        function.slot(*slot).expect("pattern slot").kind,
        MirSlotKind::Pattern
    );
}

#[test]
fn duplicate_nested_record_pattern_fields_are_rejected() {
    let model = build_model(MODULE, RECORD_PATTERN_FIXTURE);
    let lowered = lower(&model).expect("record-pattern fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let run_index = units[0].declarations.executables["run"].executable_index as usize;
    let pattern = units[0].executables[run_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::Match { arms, .. } => Some(&mut arms[0].pattern),
            _ => None,
        })
        .expect("record match arm");
    let PatternIr::Record { fields } = pattern else {
        panic!("outer record pattern")
    };
    fields.push(fields[0].clone());

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("duplicate pattern field must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("record pattern repeats field")
    ));
}

#[test]
fn for_in_item_type_must_match_the_owned_iterable_type() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let mirror_index = units[0].declarations.executables["mirror"].executable_index as usize;
    let item_type = units[0].executables[mirror_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::ForIn { item_type, .. } => Some(item_type),
            _ => None,
        })
        .expect("for item type");
    *item_type = Some(TypeRefIr::builtin("string"));

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("mismatched for item type must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("does not match iterable-derived type")
    ));

    let mut units = lowered.file_ir_units().to_vec();
    let item_type = units[0].executables[mirror_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::ForIn { item_type, .. } => Some(item_type),
            _ => None,
        })
        .expect("for item type");
    *item_type = None;
    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("missing for item type must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("has no exact item_type fact")
    ));
}

#[test]
fn map_entry_for_in_owns_exact_key_and_value_types() {
    let model = build_model(
        MODULE,
        r#"
          function sum(input: Map<string, number>) -> number {
            var total = 0
            for key, value in input {
              total = total + value
            }
            return total
          }
        "#,
    );
    let lowered = lower(&model).expect("map-entry fixture should lower");
    let function = lowered.mir_units()[0]
        .functions
        .iter()
        .find(|function| function.symbol == format!("{MODULE}.sum"))
        .expect("sum MIR");
    let facts = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            MirStmtKind::ForIn { facts, .. } => Some(facts),
            _ => None,
        })
        .expect("map-entry for facts");
    assert_eq!(
        facts.iterable_type,
        TypeRefIr::Builtin {
            name: "Map".to_string(),
            args: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number"),],
        }
    );
    let MirForInBinding::MapEntry {
        key_slot,
        key_type,
        value_slot,
        value_type,
    } = &facts.binding
    else {
        panic!("map entry binding facts")
    };
    assert_ne!(key_slot, value_slot);
    assert_eq!(key_type, &TypeRefIr::builtin("string"));
    assert_eq!(value_type, &TypeRefIr::builtin("number"));
}

#[test]
fn assert_condition_type_and_statement_span_table_are_checked() {
    let model = build_model(
        MODULE,
        r#"
          function check(condition: bool) -> bool {
            return condition
          }
        "#,
    );
    let lowered = lower(&model).expect("assert fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let check_index = units[0].declarations.executables["check"].executable_index as usize;
    // Source `assert` is legal only in test blocks, while File IR production
    // units deliberately reject test declarations. Reuse a parsed, typed bool
    // return expression as the exact Assert operand to exercise the MIR
    // contract without inventing an expression or bypassing source parsing.
    let return_index = units[0].executables[check_index]
        .body
        .statements
        .iter()
        .position(|statement| matches!(statement, StmtIr::Return { value: Some(_) }))
        .expect("typed bool return");
    let condition = match &units[0].executables[check_index].body.statements[return_index] {
        StmtIr::Return {
            value: Some(condition),
        } => *condition,
        _ => unreachable!(),
    };
    let assert_span = units[0].executables[check_index].statement_spans[return_index].clone();
    units[0].executables[check_index].body.statements.insert(
        return_index,
        StmtIr::Assert {
            condition,
            message: None,
        },
    );
    units[0].executables[check_index]
        .statement_spans
        .insert(return_index, assert_span);
    let valid = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect("typed assert fixture should build");
    let stored_span = valid[0].functions[0]
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.kind {
            MirStmtKind::Assert { .. } => statement.span.as_ref(),
            _ => None,
        })
        .expect("assert source statement span");
    assert_eq!(stored_span.source_id, 0);

    units[0].executables[check_index].expression_types[condition.expression as usize] =
        TypeRefIr::builtin("number");
    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("non-bool assert condition must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("assert condition has type")
    ));

    let mut units = lowered.file_ir_units().to_vec();
    let executable = &mut units[0].executables[check_index];
    let statement_count = executable.body.statements.len();
    executable.statement_spans.pop();
    let statement_span_count = executable.statement_spans.len();
    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("mismatched statement span table must fail closed");
    assert_eq!(
        error,
        MirBuildError::StatementSpanCountMismatch {
            module_path: MODULE.to_string(),
            symbol: format!("{MODULE}.check"),
            statement_count,
            statement_span_count,
        }
    );
}

#[test]
fn timeout_requires_a_positive_duration() {
    let model = build_model(MODULE, MIR_FIXTURE);
    let lowered = lower(&model).expect("MIR fixture should lower");
    let mut units = lowered.file_ir_units().to_vec();
    let mirror_index = units[0].declarations.executables["mirror"].executable_index as usize;
    let duration = units[0].executables[mirror_index]
        .body
        .statements
        .iter_mut()
        .find_map(|statement| match statement {
            StmtIr::Timeout { duration_ms, .. } => Some(duration_ms),
            _ => None,
        })
        .expect("timeout duration");
    *duration = 0;

    let error = build_mir_units(PACKAGE_ID, &units, model.callable_effects())
        .expect_err("zero timeout duration must fail closed");
    assert!(matches!(
        error,
        MirBuildError::InvalidControlFlow { ref message, .. }
            if message.contains("has zero duration")
    ));
}
