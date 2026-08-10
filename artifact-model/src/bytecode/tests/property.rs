//! Seeded property tests (§8): pseudo-random word streams against the
//! bounded decoder and the structural validator.
//!
//! Invariants (fixed seed set `0..=16`, deterministic in CI):
//!
//! 1. `BoundedDecoder::decode_function` never panics on any word sequence,
//!    whether the words form a legal function or any corruption shape
//!    (unknown/random opcodes, truncated or over-long operands, random high
//!    bits). Enforced with `catch_unwind`: a panic (which is what an
//!    out-of-bounds access looks like in this safe-Rust module) fails the
//!    test.
//! 2. Every decode/validate failure is one of the four structured
//!    `BytecodeDecodeError` kinds (or one of the C1–C8
//!    `StructuralValidationError` categories) — never a panic, never an
//!    artifact-controlled index access before the failing check (§4.3
//!    "decode 错误绝无 panic 路径").
//! 3. Legal seeds are deterministic: the generated artifact passes C1–C8,
//!    decode is identical across re-runs, and the validated view's decoded
//!    instructions equal a fresh decode.
//!
//! `#[cfg(feature = "fuzzing")]` additionally exports `fuzz_bytecode_decode_words` for
//! cargo-fuzz integration; see its doc comment. It is not compiled into
//! default test runs.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use super::*;

/// Fixed deterministic seed set for CI (设计 §8: 固定种子集 CI 内确定性).
const SEEDS: std::ops::RangeInclusive<u32> = 0..=16;

/// Adversarial draws per seed. Each draw is at most 64 generated words, so
/// the whole suite stays in the low single-digit seconds.
const DRAW_ITERATIONS: u32 = 128;

/// Zero-operand opcodes with no pool/reloc/table/slot references: any mix of
/// these decodes and passes C1–C8 inside the minimal shell, so a seed
/// generates a *legal* artifact deterministically.
const LEGAL_OPCODES: [u8; 10] = [0x05, 0x08, 0x14, 0x25, 0x51, 0x52, 0x53, 0x56, 0x57, 0x58];

/// Deterministic pseudo-random word generator (same LCG constants as the
/// existing `roundtrip::decode_never_panics...` smoke test).
struct WordGen {
    state: u32,
}

impl WordGen {
    fn new(seed: u32) -> Self {
        // Zero folds to a nonzero constant so the generator still cycles.
        let state = if seed == 0 { 0x9E37_79B9 } else { seed };
        Self { state }
    }

    fn next(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.state
    }

    /// Draw in `[0, bound)`.
    fn range(&mut self, bound: u32) -> u32 {
        debug_assert!(bound > 0);
        self.next() % bound
    }

    /// Operand-word draw biased toward boundary values: small in-bounds
    /// values, zero/one, `u32::MAX`, and the i32 extremes.
    fn operand(&mut self) -> u32 {
        match self.range(10) {
            0..=3 => self.range(8),
            4 => 0,
            5 => 1,
            6 => u32::MAX,
            7 => 0x7FFF_FFFF,
            8 => 0x8000_0000,
            _ => self.next(),
        }
    }
}

/// Adversarial word stream (§8: 随机 opcode、随机 operand words、截断、超长、
/// 随机高位). 1–64 words; each draw is one of:
///
/// - 40%: a valid opcode header followed by exactly its descriptor-declared
///   operand count of biased random operands;
/// - 20%: a fully random word (random high bits, `0xFF` sentinel, reserved
///   opcode values);
/// - 20%: a valid header followed by a *wrong* operand count (0..=n+1), which
///   either truncates the instruction or overflows into new instructions;
/// - 20%: a small word in `0x00..=0xFF` (valid opcodes, family gaps, `0xFF`).
fn adversarial_words(rng: &mut WordGen) -> Vec<u32> {
    let target_len = 1 + rng.range(64);
    let mut words = Vec::with_capacity(target_len as usize);
    while words.len() < target_len as usize {
        match rng.range(100) {
            0..=39 => {
                let descriptor = OPCODE_TABLE[rng.range(OPCODE_TABLE.len() as u32) as usize];
                words.push(descriptor.opcode as u32);
                for _ in 0..descriptor.operand_word_count() {
                    words.push(rng.operand());
                }
            }
            40..=59 => words.push(rng.next()),
            60..=79 => {
                let descriptor = OPCODE_TABLE[rng.range(OPCODE_TABLE.len() as u32) as usize];
                words.push(descriptor.opcode as u32);
                let operand_count = rng.range(descriptor.operand_word_count() + 2);
                for _ in 0..operand_count {
                    words.push(rng.operand());
                }
            }
            _ => words.push(rng.range(0x100)),
        }
    }
    words
}

/// Legal word stream: only zero-operand opcodes from `LEGAL_OPCODES`, 1–32
/// instructions. Decodes and validates unconditionally inside the shell.
fn legal_words(rng: &mut WordGen) -> Vec<u32> {
    let count = 1 + rng.range(32);
    (0..count)
        .map(|_| LEGAL_OPCODES[rng.range(LEGAL_OPCODES.len() as u32) as usize] as u32)
        .collect()
}

fn legal_source_map(words: &[u32]) -> Vec<SourceMapEntry> {
    let requires_source = words.iter().any(|&word| {
        let opcode = u8::try_from(word).expect("legal opcode fits in u8");
        matches!(
            opcode_contract_for(opcode)
                .expect("legal opcode has a canonical contract")
                .source,
            SourceContract::Required { .. }
        )
    });
    if requires_source {
        vec![source_map_synthetic(0, words.len() as u32)]
    } else {
        Vec::new()
    }
}

/// Minimal legal artifact shell around one function whose body is `words`:
/// no pools, no constant graph, no debug table, empty frame. Every pool,
/// relocation, table, slot or branch operand in `words` is therefore checked
/// as out of bounds or mis-typed before any index access (C5/C6/C7).
fn shell_artifact(words: Vec<u32>) -> BytecodeArtifact {
    let budget_checkpoint = descriptor_for_opcode(Opcode::BudgetCheckpoint).opcode as u32;
    let statement_entries = words
        .iter()
        .enumerate()
        .filter(|(_, word)| **word == budget_checkpoint)
        .enumerate()
        .map(|(ordinal, (pc, _))| crate::StatementEntry {
            pc: pc as u32,
            sequence_ordinal: 0,
            attribution_id: crate::StatementAttributionId::Generated {
                ordinal: ordinal as u32,
            },
            site: crate::InstructionSourceSite::Synthetic {
                reason: crate::SyntheticInstructionSiteReason::RuntimeControlFlow,
            },
        })
        .collect();
    let mut functions = BTreeMap::new();
    functions.insert(
        "module::f".to_string(),
        RelocatableBytecodeFunction {
            function_key: "module::f".to_string(),
            origin: crate::bytecode::dto::BytecodeFunctionOrigin::Executable {
                executable: crate::PackageExecutableCoordinate {
                    file_ir_identity: "file-ir:module".to_string(),
                    module_path: "module".to_string(),
                    executable_index: 0,
                },
            },
            type_parameters: Vec::new(),
            self_type_ref: None,
            words,
            relocations: Vec::new(),
            call_loan_layouts: Vec::new(),
            frame_layout: FrameLayout {
                slot_count: 0,
                slot_type_refs: Vec::new(),
                parameter_slots: Vec::new(),
                writable_local_slots: Vec::new(),
                result_count: 0,
                result_type_refs: Vec::new(),
                result_plans: Vec::new(),
                slot_plans: Vec::new(),
            },
            max_operand_depth: 0,
            effect_summary_ref: crate::PackageCallableId::new("operation:f"),
            exception_regions: Vec::new(),
            active_regions: Vec::new(),
            switch_tables: Vec::new(),
            statement_entries,
            source_map: Vec::new(),
        },
    );
    BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry: crate::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: crate::value_lifecycle_policy_identity().clone(),
        host_effect_registry: crate::host_effect_registry_identity().clone(),
        intrinsic_registry: crate::intrinsic_registry_identity().clone(),
        bytecode_identity: String::new(),
        image: BytecodeImage {
            functions,
            pools: BytecodePools::default(),
            constant_roots: std::collections::BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph::default(),
            debug_table: None,
        },
    }
}

/// Invariant 1 + 2 for the decoder: on any seeded adversarial word stream,
/// `decode_function` either succeeds with structurally consistent output or
/// fails with exactly one of the four `BytecodeDecodeError` kinds. A panic —
/// which is what an out-of-bounds access would produce in this safe-Rust
/// module — fails the test.
#[test]
fn decode_never_panics_and_fails_only_with_structured_errors() {
    let decoder = BoundedDecoder::new();
    for seed in SEEDS {
        let mut rng = WordGen::new(seed);
        for _ in 0..DRAW_ITERATIONS {
            let words = adversarial_words(&mut rng);
            match catch_unwind(AssertUnwindSafe(|| decoder.decode_function(&words))) {
                Ok(Ok(decoded)) => {
                    assert_eq!(decoded.instructions.len(), decoded.header_pcs.len());
                    for instruction in &decoded.instructions {
                        assert_eq!(
                            instruction.operand_words.len(),
                            instruction.descriptor.operand_word_count() as usize
                        );
                    }
                }
                Ok(Err(error)) => assert_structured_decode_error(&error),
                Err(_) => panic!("decode_function panicked on seed {seed}, words {words:?}"),
            }
        }
    }
}

/// Invariant 1 + 2 for the validator: on any seeded adversarial word stream
/// wrapped in the minimal legal artifact shell, `structurally_validate`
/// either succeeds or fails with one of the C1–C8 categories. The
/// catch_unwind guard is the out-of-bounds detector: any artifact-controlled
/// index access before its bounds check would panic here.
#[test]
fn validate_never_panics_and_fails_only_with_structured_errors() {
    for seed in SEEDS {
        let mut rng = WordGen::new(seed);
        for _ in 0..DRAW_ITERATIONS {
            let artifact = shell_artifact(adversarial_words(&mut rng));
            match catch_unwind(AssertUnwindSafe(|| structurally_validate(&artifact))) {
                Ok(Ok(_view)) => {}
                Ok(Err(error)) => assert_structured_validation_error(&error),
                Err(_) => panic!("structurally_validate panicked on seed {seed}"),
            }
        }
    }
}

/// Invariant 3: legal seeds validate, and decode/validate are deterministic —
/// a second decode equals the first, and the validated view carries exactly
/// the decoded instructions.
#[test]
fn legal_seed_artifacts_are_deterministic() {
    let decoder = BoundedDecoder::new();
    for seed in SEEDS {
        let mut rng = WordGen::new(seed);
        let words = legal_words(&mut rng);
        let mut artifact = shell_artifact(words.clone());
        artifact
            .image
            .functions
            .get_mut("module::f")
            .expect("legal shell function exists")
            .source_map = legal_source_map(&words);

        let view = structurally_validate(&artifact).expect("legal words must validate");
        let first = decoder.decode_function(&words).expect("legal words decode");
        let second = decoder
            .decode_function(&words)
            .expect("re-decode must succeed");
        assert_eq!(first, second, "decode must be deterministic on seed {seed}");

        let validated = &view.functions()[0];
        assert_eq!(validated.instructions, first.instructions);
        assert_eq!(validated.header_pcs, first.header_pcs);

        let view_again = structurally_validate(&artifact).expect("re-validate must succeed");
        assert_eq!(
            view, view_again,
            "validate must be deterministic on seed {seed}"
        );
    }
}

fn assert_structured_decode_error(error: &BytecodeDecodeError) {
    match error {
        BytecodeDecodeError::UnknownOpcode { .. }
        | BytecodeDecodeError::TruncatedInstruction { .. }
        | BytecodeDecodeError::ArithmeticOverflow { .. }
        | BytecodeDecodeError::LimitExceeded { .. } => {}
    }
}

fn assert_structured_validation_error(error: &StructuralValidationError) {
    match error {
        StructuralValidationError::Header { .. }
        | StructuralValidationError::Limits { .. }
        | StructuralValidationError::Arithmetic { .. }
        | StructuralValidationError::Decode { .. }
        | StructuralValidationError::Operand { .. }
        | StructuralValidationError::Target { .. }
        | StructuralValidationError::Table { .. }
        | StructuralValidationError::ConstantGraph { .. }
        | StructuralValidationError::Identity { .. } => {}
    }
}

/// cargo-fuzz entry point (设计 §8: `cfg(feature = "fuzzing")` 导出 fuzz entry fn,
/// 不进默认测试). Only reachable when the crate is compiled with the
/// `fuzzing` feature; default `cargo test`/`cargo build` never compile it.
///
/// # cargo-fuzz 接入
///
/// 1. 仓库根初始化 fuzz crate 并把本 crate 加为依赖：
///
///    ```text
///    cargo fuzz init
///    # fuzz/Cargo.toml 增加：
///    # skiff-artifact-model = { path = "../artifact-model" }
///    ```
///
/// 2. 编写 target `fuzz/fuzz_targets/bytecode_decode.rs`：
///
///    ```rust,ignore
///    #![no_main]
///    use libfuzzer_sys::fuzz_target;
///
///    fuzz_target!(|data: &[u8]| {
///        skiff_artifact_model::bytecode::tests::fuzz_bytecode_decode_words(data);
///    });
///    ```
///
/// 3. 本入口位于 `#[cfg(test)]` 的 tests 模块内，因此构建 fuzz target 时需
///    要 artifact-model 同时带 `test` cfg 与 `fuzzing` feature；前者仍通过
///    RUSTFLAGS 传给依赖，后者由 Cargo 显式启用：
///
///    ```text
///    RUSTFLAGS='--cfg test' cargo fuzz run bytecode_decode --features fuzzing -- -runs=100000
///    ```
///
///    libfuzzer 会把任何 panic（= decode 阶段的越界访问或 bug）当作 crash，
///    这正是本不变式的机器化检查。若未来 fuzz crate 落地时不希望依赖
///    `--cfg test`，替代方案是把本入口提升到 lib 层（`fuzzing` feature 下的
///    公开 fn，不在 tests 模块内），本函数体只消费公开 API
///    （`BoundedDecoder::decode_function` / `structurally_validate`），迁移
///    成本为零。
///
/// The input is interpreted as little-endian `u32` words; trailing 1–3 bytes
/// are ignored. Both decode and validate run; neither may panic.
#[cfg(feature = "fuzzing")]
pub fn fuzz_bytecode_decode_words(data: &[u8]) {
    let words: Vec<u32> = data
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    let _ = BoundedDecoder::new().decode_function(&words);
    let _ = structurally_validate(&shell_artifact(words));
}
