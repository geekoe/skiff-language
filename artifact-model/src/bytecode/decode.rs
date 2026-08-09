//! Bounded wordcode decoder (§4.1).
//!
//! Iterative (no recursion), all word access guarded by checked arithmetic,
//! all count-class limits enforced before use. Any error aborts the whole
//! function decode; there is no partial-success path and no panic path.

use crate::bytecode::limits;
use crate::bytecode::opcodes::{opcode_for, OpcodeDescriptor};

/// Decode failure (§4.3). The function key is supplied by the caller context
/// (validation wraps it into `StructuralValidationError::Decode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BytecodeDecodeError {
    /// Header word out of the opcode value range (`> 0xFF`, includes the
    /// permanent `0xFF` sentinel) or not present in the current table.
    UnknownOpcode { pc: u32, word: u32 },
    /// Instruction header valid but operand words run past the end of the
    /// function body.
    TruncatedInstruction {
        pc: u32,
        expected_words: u32,
        available: u32,
    },
    /// A checked_* arithmetic operation failed.
    ArithmeticOverflow { context: &'static str },
    /// A decode-stage resource limit was exceeded.
    LimitExceeded {
        limit: &'static str,
        actual: u64,
        max: u64,
    },
}

impl std::fmt::Display for BytecodeDecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOpcode { pc, word } => {
                write!(formatter, "unknown opcode at pc {pc}: word 0x{word:08x}")
            }
            Self::TruncatedInstruction {
                pc,
                expected_words,
                available,
            } => write!(
                formatter,
                "truncated instruction at pc {pc}: needs {expected_words} words, only {available} available"
            ),
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "arithmetic overflow: {context}")
            }
            Self::LimitExceeded {
                limit,
                actual,
                max,
            } => write!(formatter, "limit {limit} exceeded: actual {actual} > max {max}"),
        }
    }
}

impl std::error::Error for BytecodeDecodeError {}

/// One decoded instruction: header pc, its descriptor and the raw operand
/// words (branch targets stay encoded; overflow-safe target decoding is
/// `decode_branch_target`, range/header validation is C6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedInstruction {
    pub pc: u32,
    pub descriptor: &'static OpcodeDescriptor,
    pub operand_words: Vec<u32>,
}

impl DecodedInstruction {
    /// Operand word at `position` (positions are descriptor-declared).
    pub fn operand(&self, position: usize) -> u32 {
        self.operand_words[position]
    }
}

/// Output of one function decode: instructions in pc order and the ascending
/// header pc list used for target membership checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFunction {
    pub instructions: Vec<DecodedInstruction>,
    pub header_pcs: Vec<u32>,
}

/// Bounded decoder. Stateless; constructed once and reused.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoundedDecoder;

impl BoundedDecoder {
    pub const fn new() -> Self {
        Self
    }

    /// Decodes a whole function body (§4.1 steps 1–4). `words` is the raw
    /// function wordcode; the decoder validates the declared
    /// instruction-count bound independently of any earlier bounds checks.
    pub fn decode_function(&self, words: &[u32]) -> Result<DecodedFunction, BytecodeDecodeError> {
        let mut instructions = Vec::new();
        let mut header_pcs = Vec::new();
        let mut pc: u32 = 0;
        while pc < words.len() as u32 {
            let word = words[pc as usize];
            let descriptor = if word <= 0xFF {
                opcode_for(word as u8)
            } else {
                None
            }
            .ok_or(BytecodeDecodeError::UnknownOpcode { pc, word })?;
            let operand_words = descriptor.operand_word_count();
            let end = pc
                .checked_add(1)
                .and_then(|value| value.checked_add(operand_words))
                .ok_or(BytecodeDecodeError::ArithmeticOverflow {
                    context: "decode instruction extent",
                })?;
            if end > words.len() as u32 {
                return Err(BytecodeDecodeError::TruncatedInstruction {
                    pc,
                    expected_words: descriptor.instruction_word_count(),
                    available: (words.len() as u32).saturating_sub(pc),
                });
            }
            header_pcs.push(pc);
            instructions.push(DecodedInstruction {
                pc,
                descriptor,
                operand_words: words[(pc as usize + 1)..end as usize].to_vec(),
            });
            pc = end;
            if instructions.len() as u64 > limits::MAX_WORDS_PER_FUNCTION {
                return Err(BytecodeDecodeError::LimitExceeded {
                    limit: "MAX_WORDS_PER_FUNCTION",
                    actual: instructions.len() as u64,
                    max: limits::MAX_WORDS_PER_FUNCTION,
                });
            }
        }
        Ok(DecodedFunction {
            instructions,
            header_pcs,
        })
    }
}

/// Overflow-safe branch target decode (D4):
/// `targetPc = instructionHeaderPc + 1 + operandWordCount + delta`.
/// Returns `None` when the checked arithmetic overflows; range/header
/// membership is validated later (C6).
pub fn decode_branch_target(
    instruction_header_pc: u32,
    operand_word_count: u32,
    delta_word: u32,
) -> Option<u32> {
    let base = instruction_header_pc
        .checked_add(1)?
        .checked_add(operand_word_count)?;
    let delta = delta_word as i32;
    if delta >= 0 {
        base.checked_add(delta as u32)
    } else {
        base.checked_sub(delta.unsigned_abs())
    }
}
