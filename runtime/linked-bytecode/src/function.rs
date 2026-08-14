use skiff_artifact_model::{CallableEffectSummary, PackageCallableId};

use crate::{
    LinkedActiveRegion, LinkedCallLoanLayout, LinkedExceptionRegion, LinkedFrameLayout,
    LinkedInstruction, LinkedSourceMapEntry, LinkedStackMapCandidate, LinkedStatementEntry,
    LinkedSwitchTable, SpecializationKey, TypeIndex,
};

/// Exact compiler-owned declarative effect facts retained through linking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCallableEffectDeclaration {
    effect_summary_ref: PackageCallableId,
    declarative_summary: CallableEffectSummary,
}

impl LinkedCallableEffectDeclaration {
    pub fn new(
        effect_summary_ref: PackageCallableId,
        declarative_summary: CallableEffectSummary,
    ) -> Self {
        Self {
            effect_summary_ref,
            declarative_summary,
        }
    }

    pub fn effect_summary_ref(&self) -> &PackageCallableId {
        &self.effect_summary_ref
    }

    pub const fn declarative_summary(&self) -> &CallableEffectSummary {
        &self.declarative_summary
    }
}

/// Function-local tables constructed from bounded exact artifact references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFunctionTables {
    exception_regions: Box<[LinkedExceptionRegion]>,
    active_regions: Box<[LinkedActiveRegion]>,
    switch_tables: Box<[LinkedSwitchTable]>,
    call_loan_layouts: Box<[LinkedCallLoanLayout]>,
    statement_entries: Box<[LinkedStatementEntry]>,
    source_map: Box<[LinkedSourceMapEntry]>,
}

impl LinkedFunctionTables {
    pub fn new(
        exception_regions: Box<[LinkedExceptionRegion]>,
        active_regions: Box<[LinkedActiveRegion]>,
        switch_tables: Box<[LinkedSwitchTable]>,
        call_loan_layouts: Box<[LinkedCallLoanLayout]>,
        statement_entries: Box<[LinkedStatementEntry]>,
        source_map: Box<[LinkedSourceMapEntry]>,
    ) -> Self {
        Self {
            exception_regions,
            active_regions,
            switch_tables,
            call_loan_layouts,
            statement_entries,
            source_map,
        }
    }

    pub fn exception_regions(&self) -> &[LinkedExceptionRegion] {
        &self.exception_regions
    }

    pub fn active_regions(&self) -> &[LinkedActiveRegion] {
        &self.active_regions
    }

    pub fn switch_tables(&self) -> &[LinkedSwitchTable] {
        &self.switch_tables
    }

    pub fn call_loan_layouts(&self) -> &[LinkedCallLoanLayout] {
        &self.call_loan_layouts
    }

    pub fn statement_entries(&self) -> &[LinkedStatementEntry] {
        &self.statement_entries
    }

    pub fn source_map(&self) -> &[LinkedSourceMapEntry] {
        &self.source_map
    }
}

/// One concrete linked function candidate.
///
/// [`SpecializationKey`] carries the exact package build and artifact function
/// key. Candidate validation requires that build to have exactly one package
/// bytecode provenance row, retaining the validated function origin and
/// `self_type_ref` without a duplicate FileIR-origin or address field here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFunction {
    index: crate::FunctionIndex,
    key: SpecializationKey,
    instructions: Box<[LinkedInstruction]>,
    frame: LinkedFrameLayout,
    max_operand_depth: u32,
    effect: LinkedCallableEffectDeclaration,
    tables: LinkedFunctionTables,
    stack_map: LinkedStackMapCandidate,
}

impl LinkedFunction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: crate::FunctionIndex,
        key: SpecializationKey,
        instructions: Box<[LinkedInstruction]>,
        frame: LinkedFrameLayout,
        max_operand_depth: u32,
        effect: LinkedCallableEffectDeclaration,
        tables: LinkedFunctionTables,
        stack_map: LinkedStackMapCandidate,
    ) -> Self {
        Self {
            index,
            key,
            instructions,
            frame,
            max_operand_depth,
            effect,
            tables,
            stack_map,
        }
    }

    pub const fn index(&self) -> crate::FunctionIndex {
        self.index
    }

    pub const fn key(&self) -> &SpecializationKey {
        &self.key
    }

    pub fn instructions(&self) -> &[LinkedInstruction] {
        &self.instructions
    }

    pub const fn frame(&self) -> &LinkedFrameLayout {
        &self.frame
    }

    pub const fn stream_result_type_ref(&self) -> Option<TypeIndex> {
        self.frame.stream_result_type_ref()
    }

    pub const fn max_operand_depth(&self) -> u32 {
        self.max_operand_depth
    }

    pub const fn effect(&self) -> &LinkedCallableEffectDeclaration {
        &self.effect
    }

    pub fn effect_summary_ref(&self) -> &PackageCallableId {
        self.effect.effect_summary_ref()
    }

    pub const fn declarative_effect_summary(&self) -> &CallableEffectSummary {
        self.effect.declarative_summary()
    }

    pub const fn tables(&self) -> &LinkedFunctionTables {
        &self.tables
    }

    pub fn exception_regions(&self) -> &[LinkedExceptionRegion] {
        self.tables.exception_regions()
    }

    pub fn active_regions(&self) -> &[LinkedActiveRegion] {
        self.tables.active_regions()
    }

    pub fn switch_tables(&self) -> &[LinkedSwitchTable] {
        self.tables.switch_tables()
    }

    pub fn call_loan_layouts(&self) -> &[LinkedCallLoanLayout] {
        self.tables.call_loan_layouts()
    }

    pub fn statement_entries(&self) -> &[LinkedStatementEntry] {
        self.tables.statement_entries()
    }

    pub fn source_map(&self) -> &[LinkedSourceMapEntry] {
        self.tables.source_map()
    }

    pub const fn stack_map(&self) -> &LinkedStackMapCandidate {
        &self.stack_map
    }
}
