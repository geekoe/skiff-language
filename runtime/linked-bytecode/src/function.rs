use skiff_artifact_model::{CallableEffectSummary, PackageCallableId};

use crate::{
    LinkedExceptionRegion, LinkedFrameLayout, LinkedInstruction, LinkedSourceMapEntry,
    LinkedStatementEntry, LinkedSwitchTable, SpecializationKey,
};

/// Linker-declared effect facts. The summary remains untrusted until the
/// independent semantic verifier recomputes and checks it.
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

/// Function-local candidate tables. Their ordering and semantic validity are
/// deliberately left for the independent verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFunctionTables {
    exception_regions: Box<[LinkedExceptionRegion]>,
    switch_tables: Box<[LinkedSwitchTable]>,
    statement_entries: Box<[LinkedStatementEntry]>,
    source_map: Box<[LinkedSourceMapEntry]>,
}

impl LinkedFunctionTables {
    pub fn new(
        exception_regions: Box<[LinkedExceptionRegion]>,
        switch_tables: Box<[LinkedSwitchTable]>,
        statement_entries: Box<[LinkedStatementEntry]>,
        source_map: Box<[LinkedSourceMapEntry]>,
    ) -> Self {
        Self {
            exception_regions,
            switch_tables,
            statement_entries,
            source_map,
        }
    }

    pub fn exception_regions(&self) -> &[LinkedExceptionRegion] {
        &self.exception_regions
    }

    pub fn switch_tables(&self) -> &[LinkedSwitchTable] {
        &self.switch_tables
    }

    pub fn statement_entries(&self) -> &[LinkedStatementEntry] {
        &self.statement_entries
    }

    pub fn source_map(&self) -> &[LinkedSourceMapEntry] {
        &self.source_map
    }
}

/// One concrete but unverified linked function candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFunction {
    index: crate::FunctionIndex,
    key: SpecializationKey,
    instructions: Box<[LinkedInstruction]>,
    frame: LinkedFrameLayout,
    max_operand_depth: u32,
    effect: LinkedCallableEffectDeclaration,
    tables: LinkedFunctionTables,
}

impl LinkedFunction {
    pub fn new(
        index: crate::FunctionIndex,
        key: SpecializationKey,
        instructions: Box<[LinkedInstruction]>,
        frame: LinkedFrameLayout,
        max_operand_depth: u32,
        effect: LinkedCallableEffectDeclaration,
        tables: LinkedFunctionTables,
    ) -> Self {
        Self {
            index,
            key,
            instructions,
            frame,
            max_operand_depth,
            effect,
            tables,
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

    pub fn switch_tables(&self) -> &[LinkedSwitchTable] {
        self.tables.switch_tables()
    }

    pub fn statement_entries(&self) -> &[LinkedStatementEntry] {
        self.tables.statement_entries()
    }

    pub fn source_map(&self) -> &[LinkedSourceMapEntry] {
        self.tables.source_map()
    }
}
