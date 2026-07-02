use std::{collections::BTreeSet, path::Path};

use crate::{
    parsed_sources::ParsedCompilerSource,
    provider_rules::{
        collect_removed_connect_provider_violations, collect_service_native_function_violations,
        collect_service_native_type_violations,
    },
    reserved_names::validate_reserved_names,
    root_projection_validation::collect_std_root_projection_violations,
    shared::publication_error::PublicationError,
    source_name_resolution::collect_unresolved_dotted_root_violations_from_facts,
    source_rules::{
        collect_service_removed_ext_root_violations, collect_service_std_type_import_violations,
        collect_stream_emit_expression_call_violations, collect_user_function_type_violations,
    },
    NameResolutionModel,
};

pub fn validate_service_publication_sources_with_name_resolution(
    diagnostic_root: &Path,
    sources: &[ParsedCompilerSource],
    name_resolution: &NameResolutionModel,
) -> Result<(), PublicationError> {
    let mut violations = Vec::new();

    collect_user_source_authoring_violations(diagnostic_root, sources, &mut violations);
    collect_reserved_name_violations(diagnostic_root, sources, name_resolution, &mut violations);
    collect_user_function_type_source_violations(diagnostic_root, sources, &mut violations);
    collect_emit_usage_violations(diagnostic_root, sources, &mut violations);
    collect_connect_provider_source_violations(diagnostic_root, sources, &mut violations);

    if violations.is_empty() {
        return Ok(());
    }

    Err(PublicationError::ContractValidation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn collect_user_source_authoring_violations(
    diagnostic_root: &Path,
    sources: &[ParsedCompilerSource],
    violations: &mut Vec<String>,
) {
    for source in sources
        .iter()
        .filter(|source| !source.source().is_test_file)
    {
        let path = source_path(diagnostic_root, source);
        if let Err(error) = crate::test_rules::validate_no_test_declarations_in_production_source(
            &path,
            source.ast(),
        ) {
            violations.push(error.to_string());
        }
    }
}

fn collect_reserved_name_violations(
    diagnostic_root: &Path,
    sources: &[ParsedCompilerSource],
    name_resolution: &NameResolutionModel,
    violations: &mut Vec<String>,
) {
    let publication_type_names = collect_service_type_names(sources);
    collect_name_resolution_model_invariant_violations(
        diagnostic_root,
        sources,
        name_resolution,
        violations,
    );
    for source in sources {
        let path = source_path(diagnostic_root, source);
        let ast = source.ast();
        validate_reserved_names(&path, ast, violations);
        let facts = name_resolution
            .source_file_facts_by_relative_path(&source.source().relative_path)
            .expect("service name resolution model must include every production source file");
        collect_unresolved_dotted_root_violations_from_facts(&path, facts, violations);
        collect_service_removed_ext_root_violations(&path, ast, violations);
        collect_std_root_projection_violations(&path, ast, violations);
        collect_service_std_type_import_violations(&path, ast, &publication_type_names, violations);
        collect_service_native_function_violations(&path, ast, violations);
        collect_service_native_type_violations(&path, ast, violations);
        crate::alias_resolution::collect_source_alias_violations(&path, ast, violations);
    }
}

fn collect_name_resolution_model_invariant_violations(
    diagnostic_root: &Path,
    sources: &[ParsedCompilerSource],
    name_resolution: &NameResolutionModel,
    violations: &mut Vec<String>,
) {
    for source in sources {
        let Some(root) = source
            .source()
            .module_path
            .split('.')
            .next()
            .filter(|root| !root.is_empty())
        else {
            continue;
        };
        if !name_resolution.module_roots().contains(root) {
            violations.push(format!(
                "{}: internal name resolution model missing module root {root}",
                source_path(diagnostic_root, source)
            ));
        }
    }

    for alias in name_resolution.package_alias_names() {
        if !name_resolution.package_alias_entities().contains_key(alias) {
            violations.push(format!(
                "internal name resolution model missing typed package alias {alias}"
            ));
        }
    }
}

fn collect_user_function_type_source_violations(
    diagnostic_root: &Path,
    sources: &[ParsedCompilerSource],
    violations: &mut Vec<String>,
) {
    for source in sources {
        let path = source_path(diagnostic_root, source);
        collect_user_function_type_violations(&path, source.ast(), violations);
    }
}

fn collect_emit_usage_violations(
    diagnostic_root: &Path,
    sources: &[ParsedCompilerSource],
    violations: &mut Vec<String>,
) {
    for source in sources {
        let path = source_path(diagnostic_root, source);
        collect_stream_emit_expression_call_violations(&path, source.ast(), violations);
    }
}

fn collect_connect_provider_source_violations(
    diagnostic_root: &Path,
    sources: &[ParsedCompilerSource],
    violations: &mut Vec<String>,
) {
    for source in sources {
        let path = source_path(diagnostic_root, source);
        collect_removed_connect_provider_violations(&path, source.ast(), violations);
    }
}

fn collect_service_type_names(sources: &[ParsedCompilerSource]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for source in sources {
        names.extend(source.ast().types.iter().map(|ty| ty.name.clone()));
        names.extend(source.ast().aliases.iter().map(|alias| alias.name.clone()));
    }
    names
}

fn source_path(diagnostic_root: &Path, source: &ParsedCompilerSource) -> String {
    diagnostic_root
        .join(&source.source().relative_path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::{Path, PathBuf},
    };

    use crate::{
        parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile,
        NameResolutionModel,
    };

    use super::*;

    #[test]
    fn service_validation_consumes_name_resolution_package_alias_facts() {
        let source = test_source(
            "api.skiff",
            "api",
            r#"
                function run() -> string {
                  return pkg.client.call()
                }
            "#,
        );
        let sources = vec![source];
        let parsed_sources =
            parse_publication_sources(Path::new("."), &sources).expect("test sources should parse");
        let mut package_aliases = BTreeMap::new();
        package_aliases.insert("pkg".to_string(), vec!["pkg".to_string()]);
        let service_aliases = BTreeSet::new();
        let name_resolution =
            NameResolutionModel::build(&parsed_sources, &package_aliases, &service_aliases);

        validate_service_publication_sources_with_name_resolution(
            Path::new("."),
            &parsed_sources,
            &name_resolution,
        )
        .expect("package alias from name_resolution facts should not be unresolved");
    }

    #[test]
    fn service_validation_rejects_missing_package_alias_from_name_resolution_facts() {
        let source = test_source(
            "api.skiff",
            "api",
            r#"
                function run() -> string {
                  return pkg.client.call()
                }
            "#,
        );
        let sources = vec![source];
        let parsed_sources =
            parse_publication_sources(Path::new("."), &sources).expect("test sources should parse");
        let package_aliases = BTreeMap::new();
        let service_aliases = BTreeSet::new();
        let name_resolution =
            NameResolutionModel::build(&parsed_sources, &package_aliases, &service_aliases);

        let error = validate_service_publication_sources_with_name_resolution(
            Path::new("."),
            &parsed_sources,
            &name_resolution,
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("unresolved root pkg") && error.contains("pkg.client.call"),
            "expected unresolved package alias diagnostic, got:\n{error}"
        );
    }

    fn test_source(path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
        CompilerSourceFile::parse(
            PathBuf::from(path),
            module_path.to_string(),
            false,
            false,
            text.to_string(),
            path,
        )
        .expect("test source should parse")
    }
}
