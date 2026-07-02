use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::{
    parsed_sources::{is_official_std_private_module_path, ParsedCompilerSource},
    root_projection_validation::{
        collect_std_root_projection_violations,
        collect_std_root_projection_violations_with_implicit_roots,
    },
    shared::ast::SourceFile,
    shared::ast_utils::source_expressions_reference_dotted_root,
    shared::id::SKIFF_STD_PUBLICATION_ID,
    shared::prelude_registry::{prelude_registry, PRELUDE_REGISTRY_ID},
    shared::publication_error::PublicationError,
    source_name_resolution::collect_unresolved_dotted_root_violations_from_facts,
};
use compiler_input_model::{is_standard_package_id, PackageDependency};

mod import_validation;
mod reserved_validation;
mod root_validation;
mod type_block_validation;
mod type_decl_validation;
mod type_expr_validation;
mod type_name_parse;
mod type_name_validation;
mod type_validation;

pub use import_validation::implicit_std_module_roots;

use import_validation::*;
use reserved_validation::*;
use root_validation::*;
use type_block_validation::*;
use type_decl_validation::*;
use type_expr_validation::*;
use type_name_parse::*;
use type_name_validation::*;
use type_validation::*;

use super::provider_rules::{
    collect_non_std_package_native_function_violations,
    collect_non_std_package_native_type_violations, collect_removed_connect_provider_violations,
};
use super::NameResolutionModel;

pub fn validate_package_sources(
    package_id: &str,
    dependencies: &[PackageDependency],
    package_root: &Path,
    parsed_sources: &[ParsedCompilerSource],
) -> Result<(), PublicationError> {
    let mut violations = Vec::new();
    let package_type_names = package_source_type_names(parsed_sources);
    let std_implicit_projection_roots = is_standard_package_id(package_id)
        .then(|| std_package_implicit_projection_roots(parsed_sources));
    let effective_dependencies;
    let dependencies = if is_standard_package_id(package_id) {
        let mut std_dependency = PackageDependency::id(SKIFF_STD_PUBLICATION_ID);
        std_dependency.alias = Some("std".to_string());
        effective_dependencies = std::iter::once(std_dependency)
            .chain(dependencies.iter().cloned())
            .collect::<Vec<_>>();
        effective_dependencies.as_slice()
    } else {
        dependencies
    };
    let package_test_allowed_internal_imports = package_production_module_paths(parsed_sources);
    let no_allowed_internal_imports = BTreeSet::new();
    let name_resolution_package_aliases = package_name_resolution_aliases(dependencies);
    let service_dep_aliases = BTreeSet::new();
    let name_resolution = NameResolutionModel::build_with(
        parsed_sources,
        &name_resolution_package_aliases,
        &service_dep_aliases,
        None,
    );
    for parsed in parsed_sources {
        let path = package_root
            .join(&parsed.source().relative_path)
            .display()
            .to_string();
        validate_package_reserved_roots(&path, parsed.ast(), dependencies, &mut violations);
        if let Some(facts) =
            name_resolution.source_file_facts_by_relative_path(&parsed.source().relative_path)
        {
            collect_unresolved_dotted_root_violations_from_facts(&path, facts, &mut violations);
        }
        let allowed_internal_imports = if parsed.source().is_test_file {
            &package_test_allowed_internal_imports
        } else {
            &no_allowed_internal_imports
        };
        validate_package_import_dependencies(
            &path,
            parsed.ast(),
            dependencies,
            allowed_internal_imports,
            &mut violations,
        );
        if let Some(implicit_roots) = std_implicit_projection_roots.as_ref() {
            collect_std_root_projection_violations_with_implicit_roots(
                &path,
                parsed.ast(),
                implicit_roots,
                &mut violations,
            );
        } else if let Some(implicit_root) =
            implicit_std_package_root(package_id, &parsed.source().module_path)
        {
            let implicit_roots = vec![implicit_root];
            collect_std_root_projection_violations_with_implicit_roots(
                &path,
                parsed.ast(),
                &implicit_roots,
                &mut violations,
            );
        } else {
            collect_std_root_projection_violations(&path, parsed.ast(), &mut violations);
        }
        collect_removed_package_ext_root_violations(&path, parsed.ast(), &mut violations);
        collect_package_std_type_dependency_violations(
            &path,
            parsed.ast(),
            dependencies,
            &package_type_names,
            &mut violations,
        );
        collect_removed_connect_provider_violations(&path, parsed.ast(), &mut violations);
        crate::alias_resolution::collect_source_alias_violations(
            &path,
            parsed.ast(),
            &mut violations,
        );
        collect_non_std_package_native_function_violations(
            package_id,
            &path,
            parsed.ast(),
            &mut violations,
        );
        collect_non_std_package_native_type_violations(
            package_id,
            &path,
            parsed.ast(),
            &mut violations,
        );
    }

    if violations.is_empty() {
        return Ok(());
    }

    Err(PublicationError::ContractValidation {
        message: format!(
            "package {package_id} source validation failed:\n{}",
            violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    })
}

fn package_production_module_paths(parsed_sources: &[ParsedCompilerSource]) -> BTreeSet<String> {
    parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
        .map(|parsed| parsed.source().module_path.clone())
        .collect()
}

fn std_package_implicit_projection_roots(parsed_sources: &[ParsedCompilerSource]) -> Vec<String> {
    parsed_sources
        .iter()
        .filter(|parsed| !is_official_std_private_module_path(&parsed.source().module_path))
        .filter_map(|parsed| parsed.source().module_path.strip_prefix("std."))
        .filter_map(|rest| rest.split('.').next())
        .filter(|root| !root.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn collect_removed_package_ext_root_violations(
    path: &str,
    ast: &SourceFile,
    violations: &mut Vec<String>,
) {
    if ast
        .imports
        .iter()
        .any(|import| matches!(import.path.as_slice(), [root, ..] if root == "ext"))
        || source_expressions_reference_dotted_root(ast, "ext")
    {
        violations.push(format!("{path}: ext root has been removed"));
    }
}

fn package_name_resolution_aliases(
    dependencies: &[PackageDependency],
) -> BTreeMap<String, Vec<String>> {
    dependencies
        .iter()
        .map(|dependency| {
            let alias = dependency.effective_alias().to_string();
            (alias.clone(), vec![alias])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::{
        parsed_sources::parse_publication_sources, shared::id::SKIFF_STD_PUBLICATION_ID,
        source_graph::CompilerSourceFile,
    };

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
    fn official_std_private_modules_do_not_create_implicit_std_projection_roots() {
        let sources = vec![
            test_source(
                "log.skiff",
                "std.log",
                r#"
                    function leak() -> string {
                      return std.__private.helper
                    }
                "#,
            ),
            test_source(
                "helper.skiff",
                "std.__private.helper",
                r#"
                    function helper() -> string {
                      return "internal"
                    }
                "#,
            ),
        ];
        let parsed_sources =
            parse_publication_sources(Path::new("/tmp/std-private-projection"), &sources)
                .expect("sources should parse");

        let error = validate_package_sources(
            SKIFF_STD_PUBLICATION_ID,
            &[],
            Path::new("/tmp/std-private-projection"),
            &parsed_sources,
        )
        .expect_err("std.__private must not be an implicit std projection root")
        .to_string();

        assert!(
            error.contains("std.__private is not permitted as a std module root"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn package_dependency_alias_roots_are_resolved_by_name_resolution_model() {
        let sources = vec![test_source(
            "main.skiff",
            "main",
            r#"
                function run() -> string {
                  return dep.http.get()
                }
            "#,
        )];
        let parsed_sources =
            parse_publication_sources(Path::new("/tmp/pkg-alias-resolution"), &sources)
                .expect("sources should parse");
        let mut dependency = PackageDependency::id("example.com/dep");
        dependency.alias = Some("dep".to_string());

        validate_package_sources(
            "example.com/pkg",
            &[dependency],
            Path::new("/tmp/pkg-alias-resolution"),
            &parsed_sources,
        )
        .expect("package dependency alias should be available through NameResolutionModel");
    }

    #[test]
    fn official_std_public_modules_keep_internal_projection_roots() {
        let sources = vec![
            test_source(
                "log.skiff",
                "std.log",
                r#"
                    function record() -> string {
                      return std.telemetry
                    }
                "#,
            ),
            test_source(
                "telemetry.skiff",
                "std.telemetry",
                r#"
                    function emit() -> string {
                      return "ok"
                    }
                "#,
            ),
        ];
        let parsed_sources =
            parse_publication_sources(Path::new("/tmp/std-public-projection"), &sources)
                .expect("sources should parse");

        validate_package_sources(
            SKIFF_STD_PUBLICATION_ID,
            &[],
            Path::new("/tmp/std-public-projection"),
            &parsed_sources,
        )
        .expect("public std modules should keep implicit inter-module projections");
    }
}
