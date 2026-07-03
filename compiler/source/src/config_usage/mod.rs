use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde::Serialize;

use crate::parsed_sources::ParsedCompilerSource;
use crate::shared::error::{SourceLocation, SourceSpan};

use self::ast::collect_config_uses_in_ast;
use crate::shared::publication_error::PublicationError;

mod ast;
mod validation;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigUsageSeed {
    pub typed: Vec<ConfigUse>,
    pub presence: Vec<ConfigPresenceUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigUse {
    pub path: String,
    pub ty: String,
    pub required: bool,
    pub source_path: String,
    pub source_span: Option<ConfigSourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigPresenceUse {
    pub path: String,
    pub source_path: String,
    pub source_span: Option<ConfigSourceSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ConfigSourceSpan {
    pub start: ConfigSourcePosition,
    pub end: ConfigSourcePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ConfigSourcePosition {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl From<SourceSpan> for ConfigSourceSpan {
    fn from(span: SourceSpan) -> Self {
        Self {
            start: ConfigSourcePosition::from(span.start),
            end: ConfigSourcePosition::from(span.end),
        }
    }
}

impl From<SourceLocation> for ConfigSourcePosition {
    fn from(location: SourceLocation) -> Self {
        Self {
            line: location.line,
            column: location.column,
            offset: location.offset,
        }
    }
}

pub fn collect_config_usage_seed_from_parsed_sources(
    root: &Path,
    parsed_sources: &[ParsedCompilerSource],
) -> Result<ConfigUsageSeed, PublicationError> {
    let mut uses = Vec::new();
    let mut presence_uses = Vec::new();
    let mut violations = Vec::new();
    for parsed in parsed_sources {
        let source = parsed.source();
        let diagnostic_path = root.join(&source.relative_path).display().to_string();
        let source_path = source.relative_path.display().to_string();
        collect_config_uses_in_ast(
            &diagnostic_path,
            &source_path,
            parsed.ast(),
            &mut uses,
            &mut presence_uses,
            &mut violations,
        );
    }
    sort_config_uses(&mut uses);
    sort_config_presence_uses(&mut presence_uses);
    if violations.is_empty() {
        Ok(ConfigUsageSeed {
            typed: uses,
            presence: presence_uses,
        })
    } else {
        Err(PublicationError::ContractValidation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

pub fn collect_config_usage_seed_batches_from_parsed_sources(
    root: &Path,
    parsed_sources: &[ParsedCompilerSource],
    entrypoint_function_names: &[String],
) -> Result<Vec<ConfigUsageSeed>, PublicationError> {
    let excluded_function_names = entrypoint_function_names
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let function_indexes = entrypoint_function_names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let empty_function_names = BTreeSet::new();
    let mut common_seed = ConfigUsageSeed::default();
    let mut entrypoint_seeds = vec![ConfigUsageSeed::default(); entrypoint_function_names.len()];
    let mut violations = Vec::new();
    for parsed in parsed_sources {
        let source = parsed.source();
        let diagnostic_path = root.join(&source.relative_path).display().to_string();
        let source_path = source.relative_path.display().to_string();
        let split_test_functions = source.is_test_file;
        let const_strings = ast::collect_common_config_uses_in_ast(
            &diagnostic_path,
            &source_path,
            parsed.ast(),
            if split_test_functions {
                &excluded_function_names
            } else {
                &empty_function_names
            },
            &mut common_seed.typed,
            &mut common_seed.presence,
            &mut violations,
        );
        if split_test_functions {
            ast::collect_config_uses_in_ast_functions(
                &diagnostic_path,
                &source_path,
                parsed.ast(),
                &function_indexes,
                &const_strings,
                &mut entrypoint_seeds,
                &mut violations,
            );
        }
    }
    sort_config_uses(&mut common_seed.typed);
    sort_config_presence_uses(&mut common_seed.presence);
    if !violations.is_empty() {
        return Err(PublicationError::ContractValidation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }
    for seed in &mut entrypoint_seeds {
        seed.typed.extend(common_seed.typed.iter().cloned());
        seed.presence.extend(common_seed.presence.iter().cloned());
        sort_config_uses(&mut seed.typed);
        sort_config_presence_uses(&mut seed.presence);
    }
    Ok(entrypoint_seeds)
}

fn sort_config_uses(uses: &mut Vec<ConfigUse>) {
    uses.sort();
    uses.dedup();
}

fn sort_config_presence_uses(uses: &mut Vec<ConfigPresenceUse>) {
    uses.sort();
    uses.dedup();
}
