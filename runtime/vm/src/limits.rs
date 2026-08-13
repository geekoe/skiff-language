use std::num::{NonZeroU32, NonZeroUsize};

/// Trusted, finite structural limits for one VM fiber.
///
/// These values are supplied by execution policy, never by bytecode. The
/// verifier's declared frame bounds are checked against them before a frame
/// segment is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmLimits {
    max_frames: NonZeroUsize,
    max_value_slots: NonZeroUsize,
    max_segment_instructions: NonZeroU32,
}

impl VmLimits {
    pub const fn new(
        max_frames: NonZeroUsize,
        max_value_slots: NonZeroUsize,
        max_segment_instructions: NonZeroU32,
    ) -> Self {
        Self {
            max_frames,
            max_value_slots,
            max_segment_instructions,
        }
    }

    pub const fn max_frames(self) -> NonZeroUsize {
        self.max_frames
    }

    pub const fn max_value_slots(self) -> NonZeroUsize {
        self.max_value_slots
    }

    pub const fn max_segment_instructions(self) -> NonZeroU32 {
        self.max_segment_instructions
    }
}
