use skiff_artifact_model::Opcode;

/// One decoded semantic instruction. Operands remain image-local `u32`
/// words whose meaning is owned by the canonical [`Opcode`] descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedInstruction {
    opcode: Opcode,
    operands: Box<[u32]>,
    artifact_pc: u32,
}

impl LinkedInstruction {
    pub fn new(opcode: Opcode, operands: Box<[u32]>, artifact_pc: u32) -> Self {
        Self {
            opcode,
            operands,
            artifact_pc,
        }
    }

    pub const fn opcode(&self) -> Opcode {
        self.opcode
    }

    pub fn operands(&self) -> &[u32] {
        &self.operands
    }

    pub const fn artifact_pc(&self) -> u32 {
        self.artifact_pc
    }
}
