use std::collections::BTreeMap;

use crate::{
    shared::ast::{FunctionDecl, ImplDecl},
    shared::error::{CompileError, Result},
};

use super::SemanticSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableIndexEntry {
    pub declaration_name: String,
    pub symbol: String,
    pub executable_index: u32,
}

#[derive(Debug, Clone)]
pub struct ExecutableIndex {
    entries: BTreeMap<String, ExecutableIndexEntry>,
}

impl ExecutableIndex {
    pub fn source_index(source: &SemanticSource<'_>) -> Result<Self> {
        let mut builder = ExecutableIndexBuilder {
            module_path: source.module_path,
            entries: BTreeMap::new(),
            next_index: 0,
        };
        builder.collect_functions(&source.ast.functions)?;
        builder.collect_impls(&source.ast.impls)?;
        Ok(Self {
            entries: builder.entries,
        })
    }

    pub fn entry(&self, declaration_name: &str) -> Option<&ExecutableIndexEntry> {
        self.entries.get(declaration_name)
    }

    pub fn indices(&self) -> BTreeMap<String, u32> {
        self.entries
            .iter()
            .map(|(name, entry)| (name.clone(), entry.executable_index))
            .collect()
    }
}

struct ExecutableIndexBuilder<'a> {
    module_path: &'a str,
    entries: BTreeMap<String, ExecutableIndexEntry>,
    next_index: u32,
}

impl ExecutableIndexBuilder<'_> {
    fn collect_functions(&mut self, functions: &[FunctionDecl]) -> Result<()> {
        for function in functions {
            self.insert(function.name.clone())?;
        }
        Ok(())
    }

    fn collect_impls(&mut self, impls: &[ImplDecl]) -> Result<()> {
        for implementation in impls {
            for method in &implementation.method_bodies {
                self.insert(impl_method_declaration_name(
                    &implementation.target,
                    &method.name,
                ))?;
            }
        }
        Ok(())
    }

    fn insert(&mut self, declaration_name: String) -> Result<()> {
        let symbol = executable_symbol(self.module_path, &declaration_name);
        if let Some(existing) = self.entries.get(&declaration_name) {
            return Err(CompileError::Semantic(format!(
                "duplicate executable declaration `{}` in module {} conflicts between executable indexes {} and {}",
                declaration_name, self.module_path, existing.executable_index, self.next_index
            )));
        }
        let executable_index = self.next_index;
        self.next_index += 1;
        self.entries.insert(
            declaration_name.clone(),
            ExecutableIndexEntry {
                declaration_name,
                symbol,
                executable_index,
            },
        );
        Ok(())
    }
}

pub fn impl_method_declaration_name(target: &str, method_name: &str) -> String {
    format!("{target}.{method_name}")
}

pub fn executable_symbol(module_path: &str, declaration_name: &str) -> String {
    format!("{module_path}.{declaration_name}")
}
