use super::linked::{from_artifact_type_ref_in_program_ref, from_linked_ref};
use super::*;
use skiff_runtime_model::type_plan::builtins::{
    bare_type_name, builtin_plan, db_result_record_node, db_result_upsert_record_node,
    std_duration_plan, std_http_record_node,
};
use skiff_runtime_model::type_plan::RuntimeBuiltinShape;

pub(crate) fn std_runtime_builtin_node(
    name: &str,
    arg_count: usize,
) -> Option<Result<RuntimeTypeNode>> {
    std_http_record_node(name, arg_count).map(Ok)
}

pub(crate) fn native_builtin_plan(name: &str) -> Result<RuntimeTypePlan> {
    if name == "Duration" || name == "std.time.Duration" {
        return Ok(std_duration_plan());
    }
    if let Some(node) = std_runtime_builtin_node(name, 0) {
        return Ok(builtin_plan(name, node?));
    }
    let node = RuntimeBuiltinShape::of_name(name)
        .and_then(RuntimeBuiltinShape::leaf_node)
        .ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "native signature references unknown builtin type {name}"
            ))
        })?;
    Ok(builtin_plan(name, node))
}

/// Normalized input view over the three builtin entry forms. Depth accounting
/// is owned by each variant so the historical per-input semantics stay intact:
/// `Artifact` never deepens, `ArtifactInProgram` keeps the caller ctx without
/// deepening, and `Linked` deepens by 2 (the JSON `args`-array nesting the
/// reference walk encodes for builtin arguments).
pub(crate) enum PlanInput<'a> {
    Artifact {
        name: &'a str,
        args: &'a [skiff_artifact_model::TypeRefIr],
    },
    ArtifactInProgram {
        name: &'a str,
        args: &'a [skiff_artifact_model::TypeRefIr],
    },
    Linked {
        name: &'a str,
        args: &'a [LinkedTypeRef],
    },
}

impl<'a> PlanInput<'a> {
    pub(crate) fn bare_name(&self) -> &str {
        match self {
            Self::Artifact { name, .. }
            | Self::ArtifactInProgram { name, .. }
            | Self::Linked { name, .. } => bare_type_name(name),
        }
    }

    fn arg_count(&self) -> usize {
        match self {
            Self::Artifact { args, .. } => args.len(),
            Self::ArtifactInProgram { args, .. } => args.len(),
            Self::Linked { args, .. } => args.len(),
        }
    }

    fn is_array(&self) -> bool {
        match self {
            // The linked entry historically matched only the exact `Array`
            // spelling; the artifact entries matched through `bare_type_name`.
            Self::Linked { name, .. } => *name == "Array",
            _ => self.bare_name() == "Array",
        }
    }

    fn is_map(&self) -> bool {
        match self {
            Self::Linked { name, .. } => *name == "Map",
            _ => self.bare_name() == "Map",
        }
    }

    fn is_stream(&self) -> bool {
        self.bare_name() == "Stream"
    }

    pub(crate) fn recurse_arg_plan(
        &self,
        index: usize,
        ctx: Option<&PlanContext<'_>>,
    ) -> Result<RuntimeTypePlan> {
        match self {
            Self::Artifact { args, .. } => RuntimeTypePlan::from_artifact_type_ref(&args[index]),
            Self::ArtifactInProgram { args, .. } => from_artifact_type_ref_in_program_ref(
                &args[index],
                ctx.expect("artifact-in-program input requires a plan context"),
            ),
            Self::Linked { args, .. } => from_linked_ref(
                &args[index],
                &ctx.expect("linked input requires a plan context")
                    .deeper_by(2),
            ),
        }
    }
}

/// Structural Array/Map/Stream branches shared by the three builtin entries.
pub(crate) fn structural_builtin_node(
    input: &PlanInput<'_>,
    ctx: Option<&PlanContext<'_>>,
) -> Option<Result<RuntimeTypeNode>> {
    let count = input.arg_count();
    if input.is_array() && count == 1 {
        return Some(
            input
                .recurse_arg_plan(0, ctx)
                .map(|plan| RuntimeTypeNode::Array(Box::new(plan))),
        );
    }
    if input.is_map() && count == 2 {
        return Some(input.recurse_arg_plan(0, ctx).and_then(|key| {
            input
                .recurse_arg_plan(1, ctx)
                .map(|value| RuntimeTypeNode::Map {
                    key: Box::new(key),
                    value: Box::new(value),
                })
        }));
    }
    if input.is_stream() && count == 1 {
        return Some(
            input
                .recurse_arg_plan(0, ctx)
                .map(|plan| RuntimeTypeNode::Stream(Box::new(plan))),
        );
    }
    None
}

/// Single Db*Result entry shared by the three builtin entry forms. Fixed
/// records come from the model catalog; `DbUpsertResult`'s value recursion is
/// the only per-input difference and stays in [`PlanInput::recurse_arg_plan`].
pub(crate) fn db_result_node(
    input: &PlanInput<'_>,
    ctx: Option<&PlanContext<'_>>,
) -> Option<Result<RuntimeTypeNode>> {
    let root = input.bare_name();
    let count = input.arg_count();
    if count == 0 {
        if let Some(node) = db_result_record_node(root) {
            return Some(Ok(node));
        }
    }
    if root == "DbUpsertResult" && count == 1 {
        return Some(
            input
                .recurse_arg_plan(0, ctx)
                .map(db_result_upsert_record_node),
        );
    }
    None
}
