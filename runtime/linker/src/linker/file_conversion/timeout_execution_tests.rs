use super::*;
use skiff_artifact_model::{
    BlockIr, ConcurrentLaneIr, ConcurrentPlanIr, ExecutableBody, ExprIr,
    ExprRefIr as ArtifactExprRefIr, InstructionSourceSite, SourceMapSource, SourcePosition,
    SourceSpanRef, StmtIr, StmtRefIr, SyntheticInstructionSiteReason,
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
    linked_file_unit_from_assembly_artifact(unit, &|_| unreachable!())
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
fn linker_rejects_corrupt_lane_order_dependency_tail_and_body() {
    let corruptions = [
        "order",
        "forward_dependency",
        "duplicate_dependency",
        "tail_shape",
        "tail_closure",
        "missing_body",
    ];
    for corruption in corruptions {
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
        assert!(
            link(&unit).is_err(),
            "{corruption} must be rejected by strict link validation"
        );
    }
}
