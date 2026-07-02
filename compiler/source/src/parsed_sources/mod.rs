use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::{
    root_refs::{validate_source_root_refs, RootRefValidationPolicy},
    semantic::{SemanticPublication, SemanticSource, SourceOrigin},
    shared::publication_error::PublicationError,
    shared::{ast::SourceFile, source_role::PublicationSourceRole},
    source_graph::CompilerSourceFile,
};

pub use crate::root_refs::is_official_std_private_module_path;

#[cfg(test)]
#[path = "tests/alias_resolution.rs"]
mod alias_resolution_tests;

#[derive(Clone, Debug)]
pub struct ParsedCompilerSource {
    source: CompilerSourceFile,
    alias_targets: BTreeMap<String, String>,
}

impl ParsedCompilerSource {
    fn new(source: CompilerSourceFile, resolution: ParsedSourceResolution) -> Self {
        Self {
            source,
            alias_targets: resolution.alias_targets,
        }
    }

    pub fn source(&self) -> &CompilerSourceFile {
        &self.source
    }

    pub fn relative_path(&self) -> &Path {
        &self.source.relative_path
    }

    pub fn module_path(&self) -> &str {
        &self.source.module_path
    }

    pub fn source_text(&self) -> &str {
        &self.source.text
    }

    pub fn role(&self) -> PublicationSourceRole {
        self.source.role()
    }

    pub fn ast(&self) -> &SourceFile {
        &self.source.ast
    }

    pub fn alias_targets(&self) -> &BTreeMap<String, String> {
        &self.alias_targets
    }
}

pub fn parse_publication_sources(
    root: &Path,
    sources: &[CompilerSourceFile],
) -> Result<Vec<ParsedCompilerSource>, PublicationError> {
    parse_publication_sources_with_root_ref_policy(
        root,
        sources,
        RootRefValidationPolicy::parsed_publication_sources(),
    )
}

pub fn parse_publication_sources_with_root_ref_policy(
    root: &Path,
    sources: &[CompilerSourceFile],
    root_ref_policy: RootRefValidationPolicy,
) -> Result<Vec<ParsedCompilerSource>, PublicationError> {
    let parsed_sources = sources
        .iter()
        .map(|source| {
            if !source.is_test_file {
                crate::test_rules::validate_no_test_declarations_in_production_source(
                    &root.join(&source.relative_path).display().to_string(),
                    &source.ast,
                )
                .map_err(|source_error| PublicationError::Parse {
                    path: root.join(&source.relative_path).display().to_string(),
                    source: source_error,
                })?;
            }
            Ok::<CompilerSourceFile, PublicationError>(source.clone())
        })
        .collect::<Result<Vec<_>, _>>()?;
    build_parsed_sources(root, parsed_sources, root_ref_policy)
}

fn build_parsed_sources(
    root: &Path,
    sources: Vec<CompilerSourceFile>,
    root_ref_policy: RootRefValidationPolicy,
) -> Result<Vec<ParsedCompilerSource>, PublicationError> {
    validate_source_root_refs(root, &sources, root_ref_policy)?;
    let resolutions = ParsedSourceResolution::build_all(root, &sources)?;
    Ok(sources
        .into_iter()
        .zip(resolutions)
        .map(|(source, resolution)| ParsedCompilerSource::new(source, resolution))
        .collect())
}

pub fn service_semantic_publication<'a>(
    parsed_sources: &'a [ParsedCompilerSource],
) -> SemanticPublication<'a> {
    SemanticPublication::new(
        parsed_sources
            .iter()
            .map(|parsed| {
                SemanticSource::new(
                    parsed.source.relative_path.display().to_string(),
                    &parsed.source.module_path,
                    SourceOrigin::Service,
                    parsed.ast(),
                    parsed.alias_targets(),
                )
            })
            .collect(),
    )
}

pub fn package_semantic_publication<'a>(
    package_id: &'a str,
    parsed_sources: &'a [ParsedCompilerSource],
) -> SemanticPublication<'a> {
    SemanticPublication::new(
        parsed_sources
            .iter()
            .map(|parsed| {
                SemanticSource::new(
                    parsed.source.relative_path.display().to_string(),
                    &parsed.source.module_path,
                    SourceOrigin::Package { package_id },
                    parsed.ast(),
                    parsed.alias_targets(),
                )
            })
            .collect(),
    )
}

struct ParsedSourceResolution {
    alias_targets: BTreeMap<String, String>,
}

impl ParsedSourceResolution {
    fn build_all(
        root: &Path,
        sources: &[CompilerSourceFile],
    ) -> Result<Vec<Self>, PublicationError> {
        let alias_index = source_alias_index(sources);
        let mut violations = Vec::new();
        crate::alias_resolution::collect_alias_cycle_violations(
            &root.display().to_string(),
            &alias_index.qualified_aliases,
            &mut violations,
        );
        if !violations.is_empty() {
            return Err(PublicationError::ContractValidation {
                message: violations
                    .into_iter()
                    .map(|violation| format!("- {violation}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            });
        }

        Ok(sources
            .iter()
            .map(|source| Self {
                alias_targets: source_aliases_for_expansion(source, &alias_index),
            })
            .collect())
    }
}

struct SourceAliasIndex {
    qualified_aliases: BTreeMap<String, String>,
    qualified_alias_names: BTreeSet<String>,
}

fn source_alias_index(sources: &[CompilerSourceFile]) -> SourceAliasIndex {
    let mut qualified_alias_names = BTreeSet::new();
    for source in sources {
        for alias in &source.ast.aliases {
            qualified_alias_names.insert(format!("{}.{}", source.module_path, alias.name));
            if let Some((_, public_module_path)) = source.module_path.split_once('.') {
                qualified_alias_names.insert(format!("{public_module_path}.{}", alias.name));
            }
        }
    }

    let mut qualified_aliases = BTreeMap::new();
    for source in sources {
        let local_alias_names = local_alias_names(source);
        for alias in &source.ast.aliases {
            let target_type = scoped_alias_target_type(
                source,
                &alias.target_type.name,
                &local_alias_names,
                &qualified_alias_names,
            );
            qualified_aliases
                .entry(format!("{}.{}", source.module_path, alias.name))
                .or_insert_with(|| target_type.clone());
            if let Some((_, public_module_path)) = source.module_path.split_once('.') {
                qualified_aliases
                    .entry(format!("{public_module_path}.{}", alias.name))
                    .or_insert_with(|| target_type.clone());
            }
        }
    }

    SourceAliasIndex {
        qualified_aliases,
        qualified_alias_names,
    }
}

fn source_aliases_for_expansion(
    source: &CompilerSourceFile,
    alias_index: &SourceAliasIndex,
) -> BTreeMap<String, String> {
    let mut aliases = alias_index.qualified_aliases.clone();
    let local_alias_names = local_alias_names(source);
    for alias in &source.ast.aliases {
        aliases.insert(
            alias.name.clone(),
            scoped_alias_target_type(
                source,
                &alias.target_type.name,
                &local_alias_names,
                &alias_index.qualified_alias_names,
            ),
        );
    }
    aliases
}

fn scoped_alias_target_type(
    source: &CompilerSourceFile,
    raw: &str,
    local_alias_names: &BTreeSet<String>,
    qualified_alias_names: &BTreeSet<String>,
) -> String {
    crate::alias_resolution::qualify_alias_type_name(
        raw,
        local_alias_names,
        qualified_alias_names,
        &|name| format!("{}.{}", source.module_path, name),
    )
}

fn local_alias_names(source: &CompilerSourceFile) -> BTreeSet<String> {
    source
        .ast
        .aliases
        .iter()
        .map(|alias| alias.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn test_source(relative_path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
        CompilerSourceFile::parse(
            PathBuf::from(relative_path),
            module_path.to_string(),
            false,
            false,
            text.to_string(),
            relative_path,
        )
        .expect("test source should parse")
    }

    #[test]
    fn official_std_private_modules_do_not_create_root_private_alias() {
        let sources = vec![
            test_source(
                "log.skiff",
                "std.log",
                r#"
                    type LogEntry {
                      helper: root.__private.helper.HelperState
                    }
                "#,
            ),
            test_source(
                "helper.skiff",
                "std.__private.helper",
                r#"
                    type HelperState {
                      value: string
                    }
                "#,
            ),
        ];

        let error =
            match parse_publication_sources(Path::new("/tmp/std-private-root-alias"), &sources) {
                Ok(_) => panic!("std private modules must not create root.__private aliases"),
                Err(error) => error.to_string(),
            };

        assert!(
            error.contains(
                "root reference `root.__private.helper.HelperState` resolves to module `__private/helper.skiff` which does not exist"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn official_std_public_modules_keep_stripped_root_aliases() {
        let sources = vec![
            test_source(
                "log.skiff",
                "std.log",
                r#"
                    type LogEntry {
                      event: root.telemetry.Event
                    }
                "#,
            ),
            test_source(
                "telemetry.skiff",
                "std.telemetry",
                r#"
                    type Event {
                      value: string
                    }
                "#,
            ),
        ];

        parse_publication_sources(Path::new("/tmp/std-public-root-alias"), &sources)
            .expect("std public modules should keep stripped root aliases");
    }
}
