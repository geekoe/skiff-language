use std::collections::BTreeSet;

use skiff_artifact_model::{
    ConcurrentLaneIr, ConcurrentPlanIr, ExecutableBody, ExprIr, ExprRefIr, FileIrUnit,
    InstructionSourceSite, StmtIr, FILE_IR_FORMAT_VERSION, FILE_IR_OPCODE_TABLE_VERSION,
    FILE_IR_SCHEMA_VERSION,
};

pub(super) fn validate_file_ir_execution(unit: &FileIrUnit) -> anyhow::Result<()> {
    validate_generation(
        "schema",
        FILE_IR_SCHEMA_VERSION,
        unit.schema_version.as_str(),
    )?;
    validate_generation(
        "format",
        FILE_IR_FORMAT_VERSION,
        unit.ir_format_version.as_str(),
    )?;
    validate_generation(
        "opcode",
        FILE_IR_OPCODE_TABLE_VERSION,
        unit.opcode_table_version.as_str(),
    )?;

    for (index, constant) in unit.constants.iter().enumerate() {
        validate_body(unit, &constant.body, &format!("constant[{index}]"))?;
    }
    for (index, executable) in unit.executables.iter().enumerate() {
        validate_body(unit, &executable.body, &format!("executable[{index}]"))?;
    }
    Ok(())
}

fn validate_generation(field: &str, expected: &str, actual: &str) -> anyhow::Result<()> {
    if actual != expected {
        anyhow::bail!("File IR {field} version mismatch: expected {expected}, received {actual}");
    }
    Ok(())
}

fn validate_body(unit: &FileIrUnit, body: &ExecutableBody, owner: &str) -> anyhow::Result<()> {
    let mut block_labels = BTreeSet::new();
    for block in &body.blocks {
        if !block_labels.insert(block.label.as_str()) {
            anyhow::bail!("{owner} has duplicate executable block `{}`", block.label);
        }
        for statement in &block.statements {
            if statement.statement as usize >= body.statements.len() {
                anyhow::bail!(
                    "{owner} block `{}` references missing statement {}",
                    block.label,
                    statement.statement
                );
            }
        }
    }

    for statement in &body.statements {
        match statement {
            StmtIr::Timeout {
                duration_ms,
                body: timeout_body,
                site,
            } => {
                validate_duration(*duration_ms, owner, "statement timeout")?;
                validate_source_site(unit, site, owner, "statement timeout source site")?;
                validate_block_ref(&block_labels, timeout_body, owner, "statement timeout")?;
            }
            StmtIr::Concurrent { plan } => {
                validate_concurrent_plan(unit, body, plan, false, &block_labels, owner)?;
            }
            _ => {}
        }
    }

    for expression in &body.expressions {
        match expression {
            ExprIr::Timeout {
                duration_ms,
                value,
                site,
            } => {
                validate_duration(*duration_ms, owner, "value timeout")?;
                validate_source_site(unit, site, owner, "value timeout source site")?;
                validate_expr_ref(body, value, owner, "value timeout")?;
            }
            ExprIr::ValueBlock { block, result } => {
                validate_block_ref(&block_labels, block, owner, "value block")?;
                validate_expr_ref(body, result, owner, "value block tail")?;
            }
            ExprIr::ConcurrentValue { plan } => {
                validate_concurrent_plan(unit, body, plan, true, &block_labels, owner)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_duration(duration_ms: u64, owner: &str, subject: &str) -> anyhow::Result<()> {
    if duration_ms == 0 {
        anyhow::bail!("{owner} {subject} duration must be non-zero checked milliseconds");
    }
    Ok(())
}

fn validate_concurrent_plan(
    unit: &FileIrUnit,
    executable: &ExecutableBody,
    plan: &ConcurrentPlanIr,
    produces_value: bool,
    block_labels: &BTreeSet<&str>,
    owner: &str,
) -> anyhow::Result<()> {
    let plan_source = validate_source_site(unit, &plan.site, owner, "concurrent plan source site")?;
    let tail_indexes = plan
        .lanes
        .iter()
        .enumerate()
        .filter_map(|(index, lane)| matches!(lane, ConcurrentLaneIr::Tail { .. }).then_some(index))
        .collect::<Vec<_>>();
    let valid_tail_shape = if produces_value {
        tail_indexes == [plan.lanes.len().saturating_sub(1)]
    } else {
        tail_indexes.is_empty()
    };
    if !valid_tail_shape {
        anyhow::bail!("{owner} concurrent plan has invalid tail shape");
    }

    for (index, lane) in plan.lanes.iter().enumerate() {
        let source_order = lane.source_order();
        if source_order as usize != index {
            anyhow::bail!(
                "{owner} concurrent lane order is not contiguous at index {index}: {source_order}"
            );
        }
        let dependencies = lane.dependencies();
        if dependencies.windows(2).any(|pair| pair[0] >= pair[1]) {
            anyhow::bail!(
                "{owner} concurrent lane {source_order} dependencies must be strictly ordered and unique"
            );
        }
        if dependencies
            .iter()
            .any(|dependency| *dependency >= source_order)
        {
            anyhow::bail!("{owner} concurrent lane {source_order} has a forward dependency");
        }
        let lane_source =
            validate_source_site(unit, lane.site(), owner, "concurrent lane source site")?;
        if lane_source != plan_source {
            anyhow::bail!(
                "{owner} concurrent lane {source_order} source differs from its plan source"
            );
        }

        match lane {
            ConcurrentLaneIr::Statement { body, .. } | ConcurrentLaneIr::Serial { body, .. } => {
                validate_block_ref(block_labels, body, owner, "concurrent lane body")?;
            }
            ConcurrentLaneIr::Tail {
                dependencies, tail, ..
            } => {
                let expected = (0..source_order).collect::<Vec<_>>();
                if dependencies != &expected {
                    anyhow::bail!(
                        "{owner} concurrent tail dependencies do not close over all prior lanes"
                    );
                }
                validate_expr_ref(executable, tail, owner, "concurrent tail")?;
            }
        }
    }
    Ok(())
}

fn validate_block_ref(
    block_labels: &BTreeSet<&str>,
    block: &str,
    owner: &str,
    subject: &str,
) -> anyhow::Result<()> {
    if !block_labels.contains(block) {
        anyhow::bail!("{owner} {subject} references missing block `{block}`");
    }
    Ok(())
}

fn validate_expr_ref(
    body: &ExecutableBody,
    expression: &ExprRefIr,
    owner: &str,
    subject: &str,
) -> anyhow::Result<()> {
    if expression.expression as usize >= body.expressions.len() {
        anyhow::bail!(
            "{owner} {subject} references missing expression {}",
            expression.expression
        );
    }
    Ok(())
}

fn validate_source_site(
    unit: &FileIrUnit,
    site: &InstructionSourceSite,
    owner: &str,
    subject: &str,
) -> anyhow::Result<u64> {
    let InstructionSourceSite::Source { span } = site else {
        anyhow::bail!("{owner} {subject} must be an authored source site");
    };
    if !unit
        .source_map
        .sources
        .iter()
        .any(|source| source.id == span.source_id)
    {
        anyhow::bail!(
            "{owner} {subject} references unknown source {}",
            span.source_id
        );
    }
    let (Some(start), Some(end)) = (span.start.offset, span.end.offset) else {
        anyhow::bail!("{owner} {subject} requires exact source offsets");
    };
    if start >= end || (span.start.line, span.start.column) > (span.end.line, span.end.column) {
        anyhow::bail!("{owner} {subject} has an invalid source span");
    }
    Ok(span.source_id)
}
