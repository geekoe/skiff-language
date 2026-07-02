use std::{collections::HashSet, marker::PhantomData};

use crate::{
    error::{Result, RuntimeError},
    request_heap::RequestHeapLimits,
    runtime_value::HeapHandle,
    type_descriptor::RuntimeTypePlan,
};

#[derive(Clone, Debug)]
pub(super) struct MaterializeTraversal;

#[derive(Clone, Debug)]
pub(super) struct RuntimeCoerceTraversal;

pub(super) type MaterializeContext = HeapTraversalContext<MaterializeTraversal>;
pub(super) type RuntimeCoerceContext = HeapTraversalContext<RuntimeCoerceTraversal>;

#[derive(Clone, Copy, Debug)]
pub(super) struct StreamHandleScope {
    allow_current_node: bool,
    allow_runtime_owned_record_fields: bool,
}

impl StreamHandleScope {
    pub(super) fn root() -> Self {
        Self {
            allow_current_node: true,
            allow_runtime_owned_record_fields: false,
        }
    }

    pub(super) fn runtime_owned_handle_root() -> Self {
        Self {
            allow_current_node: true,
            allow_runtime_owned_record_fields: true,
        }
    }

    pub(super) fn nested() -> Self {
        Self {
            allow_current_node: false,
            allow_runtime_owned_record_fields: false,
        }
    }

    pub(super) fn allows_current_node(self) -> bool {
        self.allow_current_node
    }

    pub(super) fn record_field(self, record_plan: &RuntimeTypePlan, field_name: &str) -> Self {
        if self.allow_runtime_owned_record_fields
            && is_runtime_owned_stream_handle_field(record_plan, field_name)
        {
            Self::root()
        } else {
            Self::nested()
        }
    }
}

pub(super) const STREAM_HANDLE_SCOPE_ERROR: &str =
    "Stream handles are only allowed as top-level request-local values or fields of std/runtime-owned handle records";

fn is_runtime_owned_stream_handle_field(record_plan: &RuntimeTypePlan, field_name: &str) -> bool {
    if field_name != "body" {
        return false;
    }
    matches!(
        record_plan.named_type_name(),
        Some("HttpClientStreamHandle" | "std.http.HttpClientStreamHandle")
    )
}

pub(super) trait HeapTraversalMode {
    fn cycle_error(handle: HeapHandle) -> RuntimeError;
}

impl HeapTraversalMode for MaterializeTraversal {
    fn cycle_error(handle: HeapHandle) -> RuntimeError {
        RuntimeError::Decode(format!(
            "cannot materialize cyclic heap graph at handle {handle}"
        ))
    }
}

impl HeapTraversalMode for RuntimeCoerceTraversal {
    fn cycle_error(handle: HeapHandle) -> RuntimeError {
        RuntimeError::Decode(format!(
            "cannot coerce cyclic heap graph at handle {handle}"
        ))
    }
}

#[derive(Clone, Debug)]
pub(super) struct HeapTraversalContext<Mode> {
    active: HashSet<HeapHandle>,
    max_depth: usize,
    limits: RequestHeapLimits,
    mode: PhantomData<Mode>,
}

impl<Mode> HeapTraversalContext<Mode>
where
    Mode: HeapTraversalMode,
{
    pub(super) fn new(limits: RequestHeapLimits) -> Self {
        Self {
            active: HashSet::new(),
            max_depth: 0,
            limits,
            mode: PhantomData,
        }
    }

    pub(super) fn max_depth(&self) -> usize {
        self.max_depth
    }

    pub(super) fn check_depth(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_materialize_depth {
            return Err(RuntimeError::ResourceLimitExceeded {
                resource: "requestHeap".to_string(),
                reason: "max materialize depth".to_string(),
                limit: self.limits.max_materialize_depth,
                current: self.max_depth,
                requested_delta: depth.saturating_sub(self.max_depth),
            });
        }
        self.max_depth = self.max_depth.max(depth);
        Ok(())
    }

    pub(super) fn with_active_handle<T>(
        &mut self,
        handle: HeapHandle,
        traverse: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        if !self.active.insert(handle) {
            return Err(Mode::cycle_error(handle));
        }
        let result = traverse(self);
        self.active.remove(&handle);
        result
    }
}
