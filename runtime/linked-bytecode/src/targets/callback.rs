use skiff_artifact_model::{NativeTarget, ValueTransferPlanKind};

use crate::{
    FrameSlotIndex, FunctionIndex, HostEffectAdapterIndex, LinkedCallableSignature,
    SyntheticCallbackIndex, TypeIndex,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallbackCapture {
    slot: FrameSlotIndex,
    ty: TypeIndex,
    plan: ValueTransferPlanKind,
}

impl LinkedCallbackCapture {
    pub fn new(slot: FrameSlotIndex, ty: TypeIndex, plan: ValueTransferPlanKind) -> Self {
        Self { slot, ty, plan }
    }

    pub const fn slot(&self) -> FrameSlotIndex {
        self.slot
    }

    pub const fn ty(&self) -> TypeIndex {
        self.ty
    }

    pub const fn plan(&self) -> ValueTransferPlanKind {
        self.plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSyntheticCallbackTarget {
    index: SyntheticCallbackIndex,
    function: FunctionIndex,
    captures: Box<[LinkedCallbackCapture]>,
    signature: LinkedCallableSignature,
}

impl LinkedSyntheticCallbackTarget {
    pub fn new(
        index: SyntheticCallbackIndex,
        function: FunctionIndex,
        captures: Box<[LinkedCallbackCapture]>,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            index,
            function,
            captures,
            signature,
        }
    }

    pub const fn index(&self) -> SyntheticCallbackIndex {
        self.index
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub fn captures(&self) -> &[LinkedCallbackCapture] {
        &self.captures
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkedHostEffectAdapterTarget {
    index: HostEffectAdapterIndex,
    target: NativeTarget,
    signature: LinkedCallableSignature,
}

impl LinkedHostEffectAdapterTarget {
    pub fn new(
        index: HostEffectAdapterIndex,
        target: NativeTarget,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            index,
            target,
            signature,
        }
    }

    pub const fn index(&self) -> HostEffectAdapterIndex {
        self.index
    }

    pub const fn target(&self) -> &NativeTarget {
        &self.target
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}
