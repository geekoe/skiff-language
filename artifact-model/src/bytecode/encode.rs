//! Canonical bytecode encoders (Phase 2 emitter consumers).
//!
//! Instruction length always comes from the descriptor table
//! (`opcodes::OpcodeDescriptor::instruction_word_count`); no opcode number or
//! length is hand-written here.

use crate::bytecode::dto::BytecodeArtifact;
use crate::bytecode::opcodes::opcode_for;

/// Encoder failure. The encoder never produces a partially assembled function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    UnknownOpcode(u8),
    OperandCountMismatch {
        opcode: u8,
        expected: u32,
        actual: usize,
    },
    /// Checked length/offset arithmetic failed while assembling.
    ArithmeticOverflow {
        context: &'static str,
    },
    /// Canonical JSON serialization failed.
    CanonicalSerialization(String),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOpcode(opcode) => write!(formatter, "unknown opcode 0x{opcode:02x}"),
            Self::OperandCountMismatch {
                opcode,
                expected,
                actual,
            } => write!(
                formatter,
                "opcode 0x{opcode:02x} expects {expected} operand words, got {actual}"
            ),
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "arithmetic overflow in {context}")
            }
            Self::CanonicalSerialization(message) => {
                write!(formatter, "canonical JSON serialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

/// One already-encoded instruction (opcode + operand words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedInstruction {
    pub opcode: u8,
    pub operands: Vec<u32>,
}

impl EncodedInstruction {
    pub fn new(opcode: u8, operands: Vec<u32>) -> Self {
        Self { opcode, operands }
    }
}

/// Encodes one instruction: 1 header word + descriptor-declared operand words.
pub fn encode_instruction(opcode: u8, operands: &[u32]) -> Result<Vec<u32>, EncodeError> {
    let descriptor = opcode_for(opcode).ok_or(EncodeError::UnknownOpcode(opcode))?;
    let expected = descriptor.operand_word_count() as usize;
    if operands.len() != expected {
        return Err(EncodeError::OperandCountMismatch {
            opcode,
            expected: descriptor.operand_word_count(),
            actual: operands.len(),
        });
    }
    let mut words = Vec::with_capacity(1 + expected);
    words.push(opcode as u32);
    words.extend_from_slice(operands);
    Ok(words)
}

/// Concatenates encoded instructions into a function wordcode body.
/// Total word count is computed with checked arithmetic.
pub fn assemble_function(instructions: &[EncodedInstruction]) -> Result<Vec<u32>, EncodeError> {
    let mut total: u32 = 0;
    for instruction in instructions {
        let descriptor =
            opcode_for(instruction.opcode).ok_or(EncodeError::UnknownOpcode(instruction.opcode))?;
        let length = descriptor.instruction_word_count();
        total = total
            .checked_add(length)
            .ok_or(EncodeError::ArithmeticOverflow {
                context: "assemble_function word total",
            })?;
    }
    let mut words = Vec::with_capacity(total as usize);
    for instruction in instructions {
        words.extend(encode_instruction(
            instruction.opcode,
            &instruction.operands,
        )?);
    }
    Ok(words)
}

/// Assembles the canonical JSON bytes of a complete artifact record.
pub fn assemble_artifact(artifact: &BytecodeArtifact) -> Result<Vec<u8>, EncodeError> {
    skiff_canonical_json::canonical_json_bytes(artifact)
        .map_err(|error| EncodeError::CanonicalSerialization(error.to_string()))
}
