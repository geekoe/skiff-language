use std::sync::Arc;

use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};
use skiff_runtime_linker::DeploymentExecutionImage;

/// VM-minted authority for one exact inline projection point.
///
/// The handoff is deliberately opaque and move-only. It carries no generated
/// payload, operation/class/phase metadata, source attribution, correlation,
/// or request-local guard. A later runtime stage must consume this value and
/// independently establish those authorities before projection can occur.
///
/// ```compile_fail
/// use std::sync::Arc;
/// use skiff_runtime_linker::DeploymentExecutionImage;
/// use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};
/// use skiff_runtime_vm::VmProjectionHandoff;
///
/// fn forge(image: Arc<DeploymentExecutionImage>) -> VmProjectionHandoff {
///     VmProjectionHandoff {
///         image,
///         function: FunctionIndex::new(0),
///         instruction: InstructionIndex::new(0),
///         stack_shape: panic!("private VM stack shape"),
///         projection_sequence: 0,
///     }
/// }
/// ```
///
/// ```compile_fail
/// use skiff_runtime_vm::VmProjectionHandoff;
///
/// fn duplicate(handoff: VmProjectionHandoff) {
///     let _second = handoff.clone();
/// }
/// ```
#[must_use = "an inline VM projection handoff is unique, move-only authority"]
pub struct VmProjectionHandoff {
    image: Arc<DeploymentExecutionImage>,
    function: FunctionIndex,
    instruction: InstructionIndex,
    stack_shape: VmProjectionStackShape,
    projection_sequence: u64,
}

/// Closed dynamic VM shape sampled from the active fiber at mint time.
///
/// Keeping this representation private prevents downstream code from
/// manufacturing or partially updating a purported VM stack shape.
struct VmProjectionStackShape {
    frame_depth: usize,
    operand_height: usize,
    active_region_depth: usize,
}

impl VmProjectionHandoff {
    pub(super) fn new(
        image: Arc<DeploymentExecutionImage>,
        function: FunctionIndex,
        instruction: InstructionIndex,
        frame_depth: usize,
        operand_height: usize,
        active_region_depth: usize,
        projection_sequence: u64,
    ) -> Self {
        Self {
            image,
            function,
            instruction,
            stack_shape: VmProjectionStackShape {
                frame_depth,
                operand_height,
                active_region_depth,
            },
            projection_sequence,
        }
    }

    /// Returns the exact verified program allocation active at the VM site.
    pub const fn image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.image
    }

    /// Returns the active typed function coordinate sampled by the VM.
    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    /// Returns the active typed instruction coordinate sampled by the VM.
    pub const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    /// Returns the complete VM call-frame depth at the inline site.
    pub const fn frame_depth(&self) -> usize {
        self.stack_shape.frame_depth
    }

    /// Returns the active frame's verified operand-stack height.
    pub const fn operand_height(&self) -> usize {
        self.stack_shape.operand_height
    }

    /// Returns the complete active-region stack depth at the inline site.
    pub const fn active_region_depth(&self) -> usize {
        self.stack_shape.active_region_depth
    }

    /// Returns the monotonic sequence issued by the originating VM fiber.
    pub const fn projection_sequence(&self) -> u64 {
        self.projection_sequence
    }
}
