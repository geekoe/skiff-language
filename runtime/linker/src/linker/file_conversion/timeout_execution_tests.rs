use super::*;
use skiff_artifact_model::{
    BlockIr, ConcurrentLaneIr, ConcurrentPlanIr, ExecutableBody, ExprIr,
    ExprRefIr as ArtifactExprRefIr, InstructionSourceSite, SourceMapSource, SourcePosition,
    SourceSpanRef, StmtIr, StmtRefIr, SyntheticInstructionSiteReason,
    MAX_SAFE_EXECUTION_DURATION_MILLISECONDS,
};

fn source_site(start: u32, end: u32) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 0,
            start: SourcePosition {
                line: 1,
                column: start,
                offset: Some(start),
            },
            end: SourcePosition {
                line: 1,
                column: end,
                offset: Some(end),
            },
        },
    }
}

fn execution_file() -> artifact::FileIrUnit {
    let mut unit = artifact::FileIrUnit::empty("timeout.fixture", "source");
    unit.source_map.sources.push(SourceMapSource {
        id: 0,
        path: "timeout/fixture.skiff".to_string(),
        module_path: "timeout.fixture".to_string(),
        source_ast_hash: Some("source".to_string()),
    });
    unit.constants.push(artifact::ConstIr {
        name: "fixture".to_string(),
        ty: artifact::TypeRefIr::builtin("number"),
        body: ExecutableBody {
            blocks: vec![
                BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
                },
                BlockIr {
                    label: "timeout_body".to_string(),
                    statements: vec![],
                },
                BlockIr {
                    label: "lane_0".to_string(),
                    statements: vec![],
                },
                BlockIr {
                    label: "lane_1".to_string(),
                    statements: vec![],
                },
            ],
            statements: vec![
                StmtIr::Timeout {
                    duration_ms: 20,
                    body: "timeout_body".to_string(),
                    site: source_site(1, 8),
                },
                StmtIr::Concurrent {
                    plan: ConcurrentPlanIr {
                        lanes: vec![ConcurrentLaneIr::Statement {
                            source_order: 0,
                            dependencies: vec![],
                            body: "lane_0".to_string(),
                            site: source_site(9, 15),
                        }],
                        site: source_site(9, 15),
                    },
                },
            ],
            expressions: vec![
                ExprIr::Literal {
                    value: artifact::LiteralIr::Number {
                        value: serde_json::Number::from(1),
                    },
                },
                ExprIr::Timeout {
                    duration_ms: 30,
                    value: ArtifactExprRefIr { expression: 0 },
                    site: source_site(16, 25),
                },
                ExprIr::ConcurrentValue {
                    plan: ConcurrentPlanIr {
                        lanes: vec![
                            ConcurrentLaneIr::Statement {
                                source_order: 0,
                                dependencies: vec![],
                                body: "lane_0".to_string(),
                                site: source_site(26, 30),
                            },
                            ConcurrentLaneIr::Serial {
                                source_order: 1,
                                dependencies: vec![0],
                                body: "lane_1".to_string(),
                                site: source_site(31, 35),
                            },
                            ConcurrentLaneIr::Tail {
                                source_order: 2,
                                dependencies: vec![0, 1],
                                tail: ArtifactExprRefIr { expression: 0 },
                                site: source_site(36, 40),
                            },
                        ],
                        site: source_site(26, 40),
                    },
                },
            ],
        },
        source_span: None,
    });
    unit
}

fn link(unit: &artifact::FileIrUnit) -> anyhow::Result<LinkedFileUnit> {
    linked_file_unit_from_assembly_artifact(unit, &|_| unreachable!(), &|_| unreachable!())
}

fn set_source_id(site: &mut InstructionSourceSite, source_id: u64) {
    let InstructionSourceSite::Source { span } = site else {
        unreachable!("fixture source site must be authored");
    };
    span.source_id = source_id;
}

fn set_lane_source_id(lane: &mut ConcurrentLaneIr, source_id: u64) {
    let site = match lane {
        ConcurrentLaneIr::Statement { site, .. }
        | ConcurrentLaneIr::Serial { site, .. }
        | ConcurrentLaneIr::Tail { site, .. } => site,
    };
    set_source_id(site, source_id);
}

#[test]
fn linker_preserves_checked_timeout_and_lane_plan_exactly() {
    let linked = link(&execution_file()).expect("valid execution plan should link");
    let body = &linked.constants[0].body;

    assert!(matches!(
        &body.statements[0],
        LinkedStmtIr::Timeout {
            duration_ms: 20,
            body,
            site,
        } if body == "timeout_body" && site == &source_site(1, 8)
    ));
    assert!(matches!(
        &body.statements[1],
        LinkedStmtIr::Concurrent {
            plan: LinkedConcurrentPlanIr { lanes, site }
        } if matches!(
            lanes.as_slice(),
            [LinkedConcurrentLaneIr::Statement {
                source_order: 0,
                dependencies,
                body,
                site: lane_site,
            }] if dependencies.is_empty()
                && body == "lane_0"
                && lane_site == &source_site(9, 15)
        ) && site == &source_site(9, 15)
    ));
    assert!(matches!(
        &body.expressions[1],
        LinkedExprIr::Timeout {
            duration_ms: 30,
            value: skiff_runtime_linked_program::ExprRefIr { expression: 0 },
            site,
        } if site == &source_site(16, 25)
    ));
    assert!(matches!(
        &body.expressions[2],
        LinkedExprIr::ConcurrentValue {
            plan: LinkedConcurrentPlanIr { lanes, site }
        } if matches!(
            lanes.as_slice(),
            [
                LinkedConcurrentLaneIr::Statement {
                    source_order: 0,
                    dependencies: statement_dependencies,
                    body: statement_body,
                    ..
                },
                LinkedConcurrentLaneIr::Serial {
                    source_order: 1,
                    dependencies: serial_dependencies,
                    body: serial_body,
                    ..
                },
                LinkedConcurrentLaneIr::Tail {
                    source_order: 2,
                    dependencies: tail_dependencies,
                    tail: skiff_runtime_linked_program::ExprRefIr { expression: 0 },
                    ..
                }
            ] if statement_dependencies.is_empty()
                && statement_body == "lane_0"
                && serial_dependencies == &[0]
                && serial_body == "lane_1"
                && tail_dependencies == &[0, 1]
        ) && site == &source_site(26, 40)
    ));
}

#[test]
fn linker_rejects_old_file_ir_generations() {
    for (field, stale) in [
        ("schema", "skiff-file-ir-v8"),
        ("format", "skiff-file-ir-format-v6"),
        ("opcode", "skiff-opcode-table-v1"),
    ] {
        let mut unit = execution_file();
        match field {
            "schema" => unit.schema_version = stale.to_string(),
            "format" => unit.ir_format_version = stale.to_string(),
            "opcode" => unit.opcode_table_version = stale.to_string(),
            _ => unreachable!(),
        }
        let error = link(&unit).expect_err("old File IR generation must fail closed");
        assert!(
            error.to_string().contains(field),
            "expected {field} diagnostic, got {error:#}"
        );
    }
}

#[test]
fn linker_rejects_corrupt_timeout_duration_and_source_site() {
    let mut zero = execution_file();
    let StmtIr::Timeout { duration_ms, .. } = &mut zero.constants[0].body.statements[0] else {
        unreachable!()
    };
    *duration_ms = 0;
    assert!(link(&zero).unwrap_err().to_string().contains("duration"));

    let mut synthetic = execution_file();
    let StmtIr::Timeout { site, .. } = &mut synthetic.constants[0].body.statements[0] else {
        unreachable!()
    };
    *site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
    };
    assert!(link(&synthetic)
        .unwrap_err()
        .to_string()
        .contains("source site"));

    let mut unknown_source = execution_file();
    let StmtIr::Timeout {
        site: InstructionSourceSite::Source { span },
        ..
    } = &mut unknown_source.constants[0].body.statements[0]
    else {
        unreachable!()
    };
    span.source_id = 99;
    assert!(link(&unknown_source)
        .unwrap_err()
        .to_string()
        .contains("unknown source"));

    let mut inexact = execution_file();
    let StmtIr::Timeout {
        site: InstructionSourceSite::Source { span },
        ..
    } = &mut inexact.constants[0].body.statements[0]
    else {
        unreachable!()
    };
    span.start.offset = None;
    assert!(link(&inexact)
        .unwrap_err()
        .to_string()
        .contains("exact source offsets"));
}

#[test]
fn linker_accepts_the_maximum_safe_statement_and_value_timeout_duration() {
    let mut unit = execution_file();
    let StmtIr::Timeout { duration_ms, .. } = &mut unit.constants[0].body.statements[0] else {
        unreachable!()
    };
    *duration_ms = MAX_SAFE_EXECUTION_DURATION_MILLISECONDS;
    let ExprIr::Timeout { duration_ms, .. } = &mut unit.constants[0].body.expressions[1] else {
        unreachable!()
    };
    *duration_ms = MAX_SAFE_EXECUTION_DURATION_MILLISECONDS;

    link(&unit).expect("the maximum safe statement and value timeout durations must link");
}

#[test]
fn linker_rejects_unsafe_statement_and_value_timeout_durations_precisely() {
    for invalid in [MAX_SAFE_EXECUTION_DURATION_MILLISECONDS + 1, u64::MAX] {
        let expected = format!(
            "duration must be within 1..={MAX_SAFE_EXECUTION_DURATION_MILLISECONDS} checked milliseconds, received {invalid}"
        );

        let mut statement = execution_file();
        let StmtIr::Timeout { duration_ms, .. } = &mut statement.constants[0].body.statements[0]
        else {
            unreachable!()
        };
        *duration_ms = invalid;
        let statement_error = link(&statement)
            .expect_err("an unsafe statement timeout duration must fail closed")
            .to_string();
        assert!(
            statement_error.contains(&expected),
            "expected precise statement duration diagnostic `{expected}`, got `{statement_error}`"
        );

        let mut value = execution_file();
        let ExprIr::Timeout { duration_ms, .. } = &mut value.constants[0].body.expressions[1]
        else {
            unreachable!()
        };
        *duration_ms = invalid;
        let value_error = link(&value)
            .expect_err("an unsafe value timeout duration must fail closed")
            .to_string();
        assert!(
            value_error.contains(&expected),
            "expected precise value duration diagnostic `{expected}`, got `{value_error}`"
        );
    }
}

#[test]
fn linker_rejects_ambiguous_and_foreign_execution_source_owners() {
    let mut duplicate = execution_file();
    duplicate.source_map.sources.push(SourceMapSource {
        id: 0,
        path: "timeout/duplicate.skiff".to_string(),
        module_path: duplicate.module_path.clone(),
        source_ast_hash: Some("duplicate".to_string()),
    });
    let duplicate_error = link(&duplicate)
        .expect_err("duplicate source ids must fail closed")
        .to_string();
    assert!(
        duplicate_error.contains("references ambiguous source 0"),
        "expected ambiguous source diagnostic, got `{duplicate_error}`"
    );

    let mut foreign_timeout = execution_file();
    foreign_timeout.source_map.sources[0].module_path = "foreign.module".to_string();
    let foreign_timeout_error = link(&foreign_timeout)
        .expect_err("a timeout source owned by another module must fail closed")
        .to_string();
    assert!(
        foreign_timeout_error.contains(
            "source 0 belongs to module `foreign.module`, not File IR module `timeout.fixture`"
        ),
        "expected foreign module diagnostic, got `{foreign_timeout_error}`"
    );

    let mut foreign_plan = execution_file();
    foreign_plan.source_map.sources.push(SourceMapSource {
        id: 1,
        path: "foreign/plan.skiff".to_string(),
        module_path: "foreign.module".to_string(),
        source_ast_hash: Some("foreign".to_string()),
    });
    let StmtIr::Concurrent { plan } = &mut foreign_plan.constants[0].body.statements[1] else {
        unreachable!()
    };
    set_source_id(&mut plan.site, 1);
    set_lane_source_id(&mut plan.lanes[0], 1);
    let foreign_plan_error = link(&foreign_plan)
        .expect_err("a plan and lane sharing a foreign source id must fail closed")
        .to_string();
    assert!(
        foreign_plan_error.contains(
            "source 1 belongs to module `foreign.module`, not File IR module `timeout.fixture`"
        ),
        "expected foreign plan module diagnostic, got `{foreign_plan_error}`"
    );
}

#[test]
fn linker_rejects_corrupt_lane_order_dependency_tail_and_body() {
    let corruptions = [
        ("order", "lane order is not contiguous"),
        ("forward_dependency", "has a forward dependency"),
        (
            "duplicate_dependency",
            "dependencies must be strictly ordered and unique",
        ),
        ("tail_shape", "concurrent plan has invalid tail shape"),
        (
            "tail_closure",
            "tail dependencies do not close over all prior lanes",
        ),
        ("missing_body", "references missing block `missing`"),
    ];
    for (corruption, expected) in corruptions {
        let mut unit = execution_file();
        if corruption == "duplicate_dependency" {
            unit.constants[0].body.blocks.push(BlockIr {
                label: "lane_duplicate".to_string(),
                statements: vec![],
            });
        }
        let StmtIr::Concurrent { plan } = &mut unit.constants[0].body.statements[1] else {
            unreachable!()
        };
        match corruption {
            "order" => {
                plan.lanes[0] = ConcurrentLaneIr::Statement {
                    source_order: 1,
                    dependencies: vec![],
                    body: "lane_0".to_string(),
                    site: source_site(9, 15),
                }
            }
            "forward_dependency" => {
                plan.lanes[0] = ConcurrentLaneIr::Statement {
                    source_order: 0,
                    dependencies: vec![0],
                    body: "lane_0".to_string(),
                    site: source_site(9, 15),
                }
            }
            "duplicate_dependency" => {
                plan.lanes = vec![
                    ConcurrentLaneIr::Statement {
                        source_order: 0,
                        dependencies: vec![],
                        body: "lane_0".to_string(),
                        site: source_site(9, 10),
                    },
                    ConcurrentLaneIr::Statement {
                        source_order: 1,
                        dependencies: vec![0, 0],
                        body: "lane_duplicate".to_string(),
                        site: source_site(11, 12),
                    },
                ];
            }
            "tail_shape" => {
                plan.lanes[0] = ConcurrentLaneIr::Tail {
                    source_order: 0,
                    dependencies: vec![],
                    tail: ArtifactExprRefIr { expression: 0 },
                    site: source_site(9, 15),
                }
            }
            "tail_closure" => {
                unit.constants[0].body.statements[1] = StmtIr::Concurrent {
                    plan: ConcurrentPlanIr {
                        lanes: vec![
                            ConcurrentLaneIr::Statement {
                                source_order: 0,
                                dependencies: vec![],
                                body: "lane_0".to_string(),
                                site: source_site(9, 10),
                            },
                            ConcurrentLaneIr::Tail {
                                source_order: 1,
                                dependencies: vec![],
                                tail: ArtifactExprRefIr { expression: 0 },
                                site: source_site(11, 12),
                            },
                        ],
                        site: source_site(9, 15),
                    },
                };
                let statement = unit.constants[0].body.statements.remove(1);
                unit.constants[0].body.blocks[0]
                    .statements
                    .retain(|statement| statement.statement != 1);
                unit.constants[0]
                    .body
                    .expressions
                    .push(ExprIr::ConcurrentValue {
                        plan: match statement {
                            StmtIr::Concurrent { plan } => plan,
                            _ => unreachable!(),
                        },
                    });
            }
            "missing_body" => {
                plan.lanes[0] = ConcurrentLaneIr::Statement {
                    source_order: 0,
                    dependencies: vec![],
                    body: "missing".to_string(),
                    site: source_site(9, 15),
                }
            }
            _ => unreachable!(),
        }
        let error = link(&unit)
            .expect_err("corrupt concurrent execution plan must fail closed")
            .to_string();
        assert!(
            error.contains(expected),
            "{corruption} must report `{expected}`, got `{error}`"
        );
    }
}
