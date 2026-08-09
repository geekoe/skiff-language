//! Strongly separated image-local indices.
//!
//! The private fields prevent accidental interchange even though every index
//! has the same physical representation:
//!
//! ```compile_fail
//! use skiff_runtime_linked_bytecode::{FunctionIndex, TypeIndex};
//!
//! fn needs_type(_: TypeIndex) {}
//!
//! needs_type(FunctionIndex::new(0));
//! ```

macro_rules! image_index {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u32 {
                self.0
            }
        }
    };
}

image_index!(FunctionIndex);
image_index!(InstructionIndex);
image_index!(TypeIndex);
image_index!(ShapeIndex);
image_index!(ConstantIndex);
image_index!(ServiceOperationIndex);
image_index!(ActorMethodIndex);
image_index!(ActorCreateIndex);
image_index!(InterfaceTableIndex);
image_index!(SyntheticCallbackIndex);
image_index!(CallbackCaptureLayoutIndex);
image_index!(HostEffectAdapterIndex);
image_index!(ResumeSiteIndex);
image_index!(FrozenConstantNodeIndex);
image_index!(WritablePathIndex);
image_index!(ActiveRegionIndex);
image_index!(ExceptionRegionIndex);
image_index!(SwitchTableIndex);
image_index!(CallLoanLayoutIndex);
image_index!(IntrinsicIndex);
image_index!(ArtifactTypeIndex);
image_index!(ArtifactShapeIndex);
image_index!(ArtifactConstantIndex);
image_index!(ArtifactConstantNodeIndex);
image_index!(ArtifactWritablePathIndex);
image_index!(ArtifactCallbackCaptureIndex);
image_index!(BytecodePackageIndex);

// Function-local instruction boundary. Unlike `InstructionIndex`, this may
// equal the function's instruction count to represent an exclusive end.
image_index!(InstructionBoundaryIndex);

// Function-local frame slot index. It is kept distinct from every image
// table index for the same reason as the image-local indices above.
image_index!(FrameSlotIndex);
