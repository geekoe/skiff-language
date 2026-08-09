//! Fail-closed accessors and structured MIR construction errors.

use std::collections::BTreeSet;

use skiff_artifact_model::{CallableEffectSummary, ExprRefIr, TypeRefIr};
use skiff_compiler_core::PackageCallableIdentityError;

use super::{MirConst, MirExpression, MirFunction, MirSlot, MirUnit};

impl MirUnit {
    /// Resolves an exact executable-table index to its MIR function. MIR
    /// function order is declaration-name order, so consumers must not index
    /// `functions` directly with a File IR executable index.
    pub fn function_by_executable_index(
        &self,
        executable_index: u32,
    ) -> Result<&MirFunction, MirContractError> {
        let mut matches = self
            .functions
            .iter()
            .filter(|function| function.executable_index == executable_index);
        let function =
            matches
                .next()
                .ok_or_else(|| MirContractError::MissingExecutableFunction {
                    module_path: self.module_path.clone(),
                    executable_index,
                })?;
        if matches.next().is_some() {
            return Err(MirContractError::DuplicateExecutableFunction {
                module_path: self.module_path.clone(),
                executable_index,
            });
        }
        Ok(function)
    }

    /// Validates that executable indices are unique and dense even though the
    /// public function vector has a different deterministic ordering.
    pub fn validate_executable_indices(&self) -> Result<(), MirContractError> {
        let mut seen = BTreeSet::new();
        for function in &self.functions {
            if !seen.insert(function.executable_index) {
                return Err(MirContractError::DuplicateExecutableFunction {
                    module_path: self.module_path.clone(),
                    executable_index: function.executable_index,
                });
            }
        }
        for expected in 0..self.functions.len() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::ExecutableIndexOverflow {
                    module_path: self.module_path.clone(),
                })?;
            self.function_by_executable_index(expected)?;
        }
        Ok(())
    }

    /// Resolves a function-local `LoadConst` index to the exact graph key and
    /// type metadata owned by this MIR unit.
    pub fn constant(&self, const_index: u32) -> Result<&MirConst, MirContractError> {
        let constant = self.constants.get(const_index as usize).ok_or_else(|| {
            MirContractError::MissingConstant {
                module_path: self.module_path.clone(),
                const_index,
                constant_count: self.constants.len(),
            }
        })?;
        if constant.index != const_index {
            return Err(MirContractError::ConstantIndexMismatch {
                module_path: self.module_path.clone(),
                requested: const_index,
                stored: constant.index,
            });
        }
        Ok(constant)
    }

    /// Validates dense constant indices and unique ConstEvaluator graph keys.
    pub fn validate_constants(&self) -> Result<(), MirContractError> {
        let mut symbols = BTreeSet::new();
        for (expected, constant) in self.constants.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::ConstantIndexOverflow {
                    module_path: self.module_path.clone(),
                })?;
            if constant.index != expected {
                return Err(MirContractError::ConstantIndexMismatch {
                    module_path: self.module_path.clone(),
                    requested: expected,
                    stored: constant.index,
                });
            }
            if !symbols.insert(&constant.symbol) {
                return Err(MirContractError::DuplicateConstantSymbol {
                    module_path: self.module_path.clone(),
                    symbol: constant.symbol.clone(),
                });
            }
        }
        Ok(())
    }
}

impl MirFunction {
    /// Resolves a function-local expression reference without consulting File
    /// IR. A missing or non-canonical index is a structured contract failure.
    pub fn expression(
        &self,
        expression_ref: ExprRefIr,
    ) -> Result<&MirExpression, MirContractError> {
        let expression = self
            .expressions
            .get(expression_ref.expression as usize)
            .ok_or_else(|| MirContractError::MissingExpression {
                function: self.symbol.clone(),
                index: expression_ref.expression,
                expression_count: self.expressions.len(),
            })?;
        if expression.index != expression_ref.expression {
            return Err(MirContractError::ExpressionIndexMismatch {
                function: self.symbol.clone(),
                requested: expression_ref.expression,
                stored: expression.index,
            });
        }
        Ok(expression)
    }

    /// Validates the complete function-owned expression table, including
    /// entries not reached by the current CFG.
    pub fn validate_expression_indices(&self) -> Result<(), MirContractError> {
        for (expected, expression) in self.expressions.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::ExpressionIndexOverflow {
                    function: self.symbol.clone(),
                })?;
            if expression.index != expected {
                return Err(MirContractError::ExpressionIndexMismatch {
                    function: self.symbol.clone(),
                    requested: expected,
                    stored: expression.index,
                });
            }
        }
        Ok(())
    }

    /// Resolves a slot by its function-local index. Slot vector order is part
    /// of the MIR contract and is checked rather than inferred.
    pub fn slot(&self, slot: u32) -> Result<&MirSlot, MirContractError> {
        let entry = self
            .slots
            .get(slot as usize)
            .ok_or_else(|| MirContractError::MissingSlot {
                function: self.symbol.clone(),
                slot,
                slot_count: self.slots.len(),
            })?;
        if entry.slot != slot {
            return Err(MirContractError::SlotIndexMismatch {
                function: self.symbol.clone(),
                requested: slot,
                stored: entry.slot,
            });
        }
        Ok(entry)
    }

    /// Returns the source-owned static slot type. Future emitters must use
    /// this checked surface and fail when lowering left a slot untyped; they
    /// must not infer a replacement type from expressions or File IR.
    pub fn slot_type(&self, slot: u32) -> Result<&TypeRefIr, MirContractError> {
        let entry = self.slot(slot)?;
        entry
            .ty
            .as_ref()
            .ok_or_else(|| MirContractError::MissingSlotType {
                function: self.symbol.clone(),
                slot,
                name: entry.name.clone(),
            })
    }

    /// Validates that every slot exposed to emission has an exact static type.
    pub fn validate_slot_types(&self) -> Result<(), MirContractError> {
        for (expected, slot) in self.slots.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::SlotIndexOverflow {
                    function: self.symbol.clone(),
                })?;
            if slot.slot != expected {
                return Err(MirContractError::SlotIndexMismatch {
                    function: self.symbol.clone(),
                    requested: expected,
                    stored: slot.slot,
                });
            }
            self.slot_type(expected)?;
        }
        Ok(())
    }

    /// Conservative pending fact derived from the single owned effect
    /// summary. Unknown analysis can never grant a synchronous optimization.
    pub fn may_pending(&self) -> bool {
        match &self.effect_summary {
            CallableEffectSummary::Analyzed { effects } => effects.may_pending(),
            CallableEffectSummary::Unknown { .. } => true,
        }
    }
}

/// A fail-closed lookup/validation failure in an already-built MIR function.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MirContractError {
    #[error("MIR unit `{module_path}` has no function for executable index {executable_index}")]
    MissingExecutableFunction {
        module_path: String,
        executable_index: u32,
    },
    #[error(
        "MIR unit `{module_path}` has multiple functions for executable index {executable_index}"
    )]
    DuplicateExecutableFunction {
        module_path: String,
        executable_index: u32,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX functions")]
    ExecutableIndexOverflow { module_path: String },
    #[error(
        "MIR unit `{module_path}` has no constant {const_index} (constant count {constant_count})"
    )]
    MissingConstant {
        module_path: String,
        const_index: u32,
        constant_count: usize,
    },
    #[error(
        "MIR unit `{module_path}` constant lookup {requested} found non-canonical stored index {stored}"
    )]
    ConstantIndexMismatch {
        module_path: String,
        requested: u32,
        stored: u32,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX constants")]
    ConstantIndexOverflow { module_path: String },
    #[error("MIR unit `{module_path}` repeats constant symbol `{symbol}`")]
    DuplicateConstantSymbol { module_path: String, symbol: String },
    #[error(
        "MIR function `{function}` has no expression {index} (expression count {expression_count})"
    )]
    MissingExpression {
        function: String,
        index: u32,
        expression_count: usize,
    },
    #[error(
        "MIR function `{function}` expression lookup {requested} found non-canonical stored index {stored}"
    )]
    ExpressionIndexMismatch {
        function: String,
        requested: u32,
        stored: u32,
    },
    #[error("MIR function `{function}` has more than u32::MAX expressions")]
    ExpressionIndexOverflow { function: String },
    #[error("MIR function `{function}` has no slot {slot} (slot count {slot_count})")]
    MissingSlot {
        function: String,
        slot: u32,
        slot_count: usize,
    },
    #[error(
        "MIR function `{function}` slot lookup {requested} found non-canonical stored index {stored}"
    )]
    SlotIndexMismatch {
        function: String,
        requested: u32,
        stored: u32,
    },
    #[error("MIR function `{function}` has more than u32::MAX slots")]
    SlotIndexOverflow { function: String },
    #[error("MIR function `{function}` slot {slot} (`{name}`) has no static type")]
    MissingSlotType {
        function: String,
        slot: u32,
        name: String,
    },
    #[error(
        "MIR function `{function}` block at position {expected} stores non-canonical id {stored}"
    )]
    BlockIndexMismatch {
        function: String,
        expected: u32,
        stored: u32,
    },
    #[error("MIR function `{function}` has more than u32::MAX blocks")]
    BlockIndexOverflow { function: String },
    #[error("MIR function `{function}` block {block} references missing successor {successor}")]
    MissingSuccessorBlock {
        function: String,
        block: u32,
        successor: u32,
    },
}

/// A structured failure while converting File IR plus source facts into MIR.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MirBuildError {
    #[error(
        "MIR unit `{module_path}` has {declaration_count} executable declarations but {executable_count} executable bodies"
    )]
    ExecutableCountMismatch {
        module_path: String,
        declaration_count: usize,
        executable_count: usize,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX executable bodies")]
    ExecutableIndexOverflow { module_path: String },
    #[error(
        "MIR unit `{module_path}` executable index {executable_index} is owned by both `{first_declaration}` and `{duplicate_declaration}`"
    )]
    DuplicateExecutableIndex {
        module_path: String,
        executable_index: u32,
        first_declaration: String,
        duplicate_declaration: String,
    },
    #[error(
        "MIR build for {module_path}::{declaration_name} references missing executable index {executable_index}"
    )]
    MissingExecutable {
        module_path: String,
        declaration_name: String,
        executable_index: u32,
    },
    #[error(
        "MIR executable declaration `{declaration_name}` in `{module_path}` stores symbol `{stored_symbol}`, expected `{expected_symbol}`"
    )]
    ExecutableDeclarationSymbolMismatch {
        module_path: String,
        declaration_name: String,
        expected_symbol: String,
        stored_symbol: String,
    },
    #[error(
        "MIR executable declaration `{declaration_name}` in `{module_path}` names `{declaration_symbol}` but its body names `{executable_symbol}`"
    )]
    ExecutableSymbolMismatch {
        module_path: String,
        declaration_name: String,
        declaration_symbol: String,
        executable_symbol: String,
    },
    #[error(
        "MIR unit `{module_path}` has {declaration_count} constant declarations but {constant_count} constant bodies"
    )]
    ConstantCountMismatch {
        module_path: String,
        declaration_count: usize,
        constant_count: usize,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX constants")]
    ConstantIndexOverflow { module_path: String },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` references index {const_index}, but only {constant_count} bodies exist"
    )]
    ConstantIndexOutOfBounds {
        module_path: String,
        declaration_name: String,
        const_index: u32,
        constant_count: usize,
    },
    #[error(
        "MIR constant declaration `{duplicate_declaration}` in `{module_path}` duplicates constant index {const_index}"
    )]
    DuplicateConstantIndex {
        module_path: String,
        const_index: u32,
        duplicate_declaration: String,
    },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` points to body `{constant_name}` at index {const_index}"
    )]
    ConstantNameMismatch {
        module_path: String,
        declaration_name: String,
        constant_name: String,
        const_index: u32,
    },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` stores symbol `{stored_symbol}`, expected `{expected_symbol}`"
    )]
    ConstantSymbolMismatch {
        module_path: String,
        declaration_name: String,
        expected_symbol: String,
        stored_symbol: String,
    },
    #[error("MIR unit `{module_path}` repeats constant symbol `{symbol}`")]
    DuplicateConstantSymbol { module_path: String, symbol: String },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` disagrees with its body {fact}"
    )]
    ConstantFactMismatch {
        module_path: String,
        declaration_name: String,
        fact: &'static str,
    },
    #[error("MIR unit `{module_path}` has no declaration for dense constant index {const_index}")]
    MissingConstantIndex {
        module_path: String,
        const_index: u32,
    },
    #[error(
        "MIR build requires source-owned callable effect facts for {module_path}::{declaration_name}"
    )]
    MissingCallableEffect {
        module_path: String,
        declaration_name: String,
    },
    #[error(
        "MIR function `{symbol}` in `{module_path}` has {expression_count} expressions but {expression_type_count} expression types"
    )]
    ExpressionTypeCountMismatch {
        module_path: String,
        symbol: String,
        expression_count: usize,
        expression_type_count: usize,
    },
    #[error("MIR function `{symbol}` in `{module_path}` has more than u32::MAX expressions")]
    ExpressionIndexOverflow { module_path: String, symbol: String },
    #[error(
        "failed to construct package callable identity for MIR function `{symbol}` in `{module_path}` (package `{package_id}`): {source}"
    )]
    CallableIdentity {
        package_id: String,
        module_path: String,
        symbol: String,
        #[source]
        source: PackageCallableIdentityError,
    },
    #[error("invalid MIR control flow for `{symbol}` in `{module_path}`: {message}")]
    InvalidControlFlow {
        module_path: String,
        symbol: String,
        message: String,
    },
    #[error("invalid MIR liveness input for `{symbol}` in `{module_path}`: {source}")]
    Liveness {
        module_path: String,
        symbol: String,
        #[source]
        source: MirContractError,
    },
    #[error("invalid owned MIR unit contract for `{module_path}`: {source}")]
    InvalidUnitContract {
        module_path: String,
        #[source]
        source: MirContractError,
    },
}
