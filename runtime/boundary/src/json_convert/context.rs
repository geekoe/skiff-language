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
    allow_internal_task_ref: bool,
    ignore_extra_record_fields: bool,
}

impl StreamHandleScope {
    pub(super) fn root() -> Self {
        Self {
            allow_current_node: true,
            allow_runtime_owned_record_fields: false,
            allow_internal_task_ref: false,
            ignore_extra_record_fields: false,
        }
    }

    pub(super) fn runtime_owned_handle_root() -> Self {
        Self {
            allow_current_node: true,
            allow_runtime_owned_record_fields: true,
            allow_internal_task_ref: false,
            ignore_extra_record_fields: false,
        }
    }

    pub(super) fn nested() -> Self {
        Self {
            allow_current_node: false,
            allow_runtime_owned_record_fields: false,
            allow_internal_task_ref: false,
            ignore_extra_record_fields: false,
        }
    }

    /// Marks this traversal as a DB contract view decode: record fields the
    /// engine view does not declare are ignored instead of rejected, because
    /// the host writes the full document shape into the shared collection.
    pub(super) fn with_ignore_extra_record_fields(mut self) -> Self {
        self.ignore_extra_record_fields = true;
        self
    }

    pub(super) fn ignores_extra_record_fields(self) -> bool {
        self.ignore_extra_record_fields
    }

    /// Marks this traversal as the owner-internal DB lane, where opaque
    /// `std.task.TaskRef` handles round-trip as their canonical strings
    /// (`doc/reference/dispatch.md` §3 DB stored field contract).
    pub(super) fn with_internal_task_ref(mut self) -> Self {
        self.allow_internal_task_ref = true;
        self
    }

    pub(super) fn allows_current_node(self) -> bool {
        self.allow_current_node
    }

    pub(super) fn allows_internal_task_ref(self) -> bool {
        self.allow_internal_task_ref
    }

    pub(super) fn record_field(self, record_plan: &RuntimeTypePlan, field_name: &str) -> Self {
        if self.allow_runtime_owned_record_fields
            && is_runtime_owned_stream_handle_field(record_plan, field_name)
        {
            Self::root().with_internal_task_ref_if(self.allow_internal_task_ref)
        } else {
            Self::nested().with_internal_task_ref_if(self.allow_internal_task_ref)
        }
        .with_ignore_extra_fields_if(self.ignore_extra_record_fields)
    }
}

impl StreamHandleScope {
    fn with_internal_task_ref_if(self, enabled: bool) -> Self {
        if enabled {
            self.with_internal_task_ref()
        } else {
            self
        }
    }

    pub(super) fn with_ignore_extra_fields_if(self, enabled: bool) -> Self {
        if enabled {
            self.with_ignore_extra_record_fields()
        } else {
            self
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
    allow_internal_task_ref: bool,
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
            allow_internal_task_ref: false,
            mode: PhantomData,
        }
    }

    /// Marks this traversal as the owner-internal DB lane, where opaque
    /// `std.task.TaskRef` handles round-trip as their canonical strings.
    pub(super) fn with_internal_task_ref(mut self) -> Self {
        self.allow_internal_task_ref = true;
        self
    }

    pub(super) fn allows_internal_task_ref(&self) -> bool {
        self.allow_internal_task_ref
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
