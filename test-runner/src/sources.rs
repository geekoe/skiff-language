use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use skiff_compiler::{
    collect_source_tree,
    test_support::{
        is_friend_test_file_for_production, production_friend_match_for_test_file,
        validate_no_test_declarations_in_production_source, FriendProductionMatch,
        TestPackageManifest as PackageManifest, PACKAGE_CONFIG_FILE,
    },
    SERVICE_CONFIG_FILE,
};
use skiff_syntax::ast::{Block, FunctionDecl, SourceFile as AstSourceFile, TypeRef};
use skiff_syntax::parser::{parse_source, parse_source_with_bodies_tolerant};

use super::{
    root_paths::{module_path_for_package_production_source, package_module_path},
    PackageTestCase, PackageTestSource, ParsedSource, PrivateVisibilityScope, SkiffTestError,
    TestCase,
};

pub(super) fn read_package_test_sources(
    input: &Path,
    package_root: &Path,
    input_is_file: bool,
    manifest: &PackageManifest,
    export_sources: &BTreeMap<PathBuf, String>,
) -> Result<Vec<PackageTestSource>, SkiffTestError> {
    let mut paths = Vec::new();
    if input_is_file {
        paths.push(input.to_path_buf());
        if !is_test_file_path(input) {
            paths.extend(friend_test_file_paths(input)?);
        }
    } else {
        collect_package_test_paths(input, &mut paths)?;
        paths.sort();
    }
    let explicit_ordinary_file = input_is_file && !is_test_file_path(input);
    let mut sources = paths
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("skiff"))
        .map(|path| read_package_test_source(&path, package_root, manifest, export_sources))
        .filter_map(|result| match result {
            Ok(Some(source)) => Some(Ok(source)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if explicit_ordinary_file {
        sources
            .retain(|source| !source.is_test_file || source.ast.test_default_run.unwrap_or(true));
    }
    Ok(sources)
}
pub(super) fn collect_package_test_paths(
    root: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), SkiffTestError> {
    for entry in fs::read_dir(root).map_err(|source| SkiffTestError::ReadSource {
        path: root.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| SkiffTestError::ReadSource {
            path: root.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| SkiffTestError::ReadSource {
                path: path.display().to_string(),
                source,
            })?;
        if file_type.is_dir() {
            if !should_skip_package_dir(&path) {
                collect_package_test_paths(&path, paths)?;
            }
        } else if file_type.is_file() {
            paths.push(path);
        }
    }
    Ok(())
}
pub(super) fn read_package_test_source(
    path: &Path,
    package_root: &Path,
    manifest: &PackageManifest,
    export_sources: &BTreeMap<PathBuf, String>,
) -> Result<Option<PackageTestSource>, SkiffTestError> {
    let text = fs::read_to_string(path).map_err(|source| SkiffTestError::ReadSource {
        path: path.display().to_string(),
        source,
    })?;
    let ast = parse_source(&text).map_err(|source| SkiffTestError::Parse {
        path: path.display().to_string(),
        source,
    })?;
    let relative_path = path
        .strip_prefix(package_root)
        .unwrap_or(path)
        .to_path_buf();
    let is_test_file = is_test_file_path(&relative_path);
    if !is_test_file {
        validate_no_test_declarations_in_production_source(&path.display().to_string(), &ast)
            .map_err(|source| SkiffTestError::Parse {
                path: path.display().to_string(),
                source,
            })?;
        return Ok(None);
    }
    if ast.tests.is_empty() {
        return Ok(None);
    }
    let friend_relative_path = package_friend_relative_path(path, package_root)?;
    let friend_module_path = friend_relative_path.as_deref().map(|relative| {
        module_path_for_package_production_source(manifest, relative, export_sources)
    });
    let module_path = package_module_path(
        manifest,
        &relative_path,
        friend_relative_path.as_deref(),
        is_test_file,
        export_sources,
    );
    Ok(Some(PackageTestSource {
        relative_path,
        module_path,
        is_test_file,
        text,
        ast,
        synthetic_imports: BTreeSet::new(),
        friend_module_path,
    }))
}
pub(super) fn read_package_production_sources(
    manifest: &PackageManifest,
    package_root: &Path,
    export_sources: &BTreeMap<PathBuf, String>,
) -> Result<Vec<PackageTestSource>, SkiffTestError> {
    let mut paths = Vec::new();
    collect_package_production_paths(package_root, package_root, &mut paths)?;
    paths.sort();
    let mut sources = Vec::new();
    for relative_path in paths {
        let path = package_root.join(&relative_path);
        let text = fs::read_to_string(&path).map_err(|source| SkiffTestError::ReadSource {
            path: path.display().to_string(),
            source,
        })?;
        let ast = parse_source(&text).map_err(|source| SkiffTestError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        validate_no_test_declarations_in_production_source(&path.display().to_string(), &ast)
            .map_err(|source| SkiffTestError::Parse {
                path: path.display().to_string(),
                source,
            })?;
        sources.push(PackageTestSource {
            module_path: module_path_for_package_production_source(
                manifest,
                &relative_path,
                export_sources,
            ),
            relative_path,
            is_test_file: false,
            text,
            ast,
            synthetic_imports: BTreeSet::new(),
            friend_module_path: None,
        });
    }
    Ok(sources)
}
pub(super) fn collect_package_production_paths(
    package_root: &Path,
    current: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), SkiffTestError> {
    for entry in fs::read_dir(current).map_err(|source| SkiffTestError::ReadSource {
        path: current.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| SkiffTestError::ReadSource {
            path: current.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| SkiffTestError::ReadSource {
                path: path.display().to_string(),
                source,
            })?;
        if file_type.is_dir() {
            if !should_skip_package_production_dir(&path) {
                collect_package_production_paths(package_root, &path, paths)?;
            }
        } else if file_type.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("skiff")
            && !is_test_file_path(&path)
        {
            paths.push(
                path.strip_prefix(package_root)
                    .expect("package source is below package root")
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}
pub(super) fn should_skip_package_production_dir(path: &Path) -> bool {
    should_skip_package_dir(path)
}
pub(super) fn service_test_runtime_module_path(source: &ParsedSource) -> String {
    if source.source.is_test_file && !source.source.module_path.ends_with(".__test") {
        // Service test sources carry their production module path (e.g. `api.client`)
        // and get the synthetic `.__test` operation module appended here. Flattened
        // package test sources already carry their `.__test` module path, so guard
        // against appending it twice.
        format!("{}.__test", source.source.module_path)
    } else {
        source.source.module_path.clone()
    }
}
pub(super) fn collect_package_test_cases(sources: &[PackageTestSource]) -> Vec<PackageTestCase> {
    let mut cases = Vec::new();
    let mut next_index = 0usize;
    for source in sources {
        for (test_index, test) in source.ast.tests.iter().enumerate() {
            let function_name = format!("__skiff_test_{}", next_index);
            next_index += 1;
            cases.push(PackageTestCase {
                module_path: source.module_path.clone(),
                name: test.name.clone(),
                test_index,
                source: source.clone(),
                function_name,
            });
        }
    }
    cases
}
pub(super) fn package_test_ast(
    ast: &AstSourceFile,
    test_index: usize,
    function_name: &str,
) -> AstSourceFile {
    package_test_ast_for_cases(ast, std::iter::once((test_index, function_name)))
}

pub(super) fn package_test_ast_for_cases<'a>(
    ast: &AstSourceFile,
    tests: impl IntoIterator<Item = (usize, &'a str)>,
) -> AstSourceFile {
    let functions = tests
        .into_iter()
        .map(|(test_index, function_name)| {
            let test = ast
                .tests
                .iter()
                .nth(test_index)
                .expect("test case came from source AST");
            (
                ast.source_spans.tests.get(test_index).cloned(),
                FunctionDecl {
                    exported: false,
                    name: function_name.to_string(),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    return_type: TypeRef {
                        name: "void".to_string(),
                    },
                    body: Block {
                        statements: test.body.statements.clone(),
                    },
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                    span: test.span,
                },
            )
        })
        .collect::<Vec<_>>();
    let mut ast = ast.clone();
    ast.tests.clear();
    ast.test_default_run = None;
    ast.source_spans.tests.clear();
    for (test_spans, function) in functions {
        if let Some(test_spans) = test_spans {
            ast.source_spans.functions.push(test_spans);
        }
        ast.functions.push(function);
    }
    ast
}
pub(super) fn friend_test_file_paths(path: &Path) -> Result<Vec<PathBuf>, SkiffTestError> {
    let Some(parent) = path.parent() else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(parent).map_err(|source| SkiffTestError::ReadSource {
        path: parent.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| SkiffTestError::ReadSource {
            path: parent.display().to_string(),
            source,
        })?;
        let candidate = entry.path();
        if candidate.is_file() && is_friend_test_file_for_production(&candidate, path) {
            paths.push(candidate);
        }
    }
    paths.sort();
    Ok(paths)
}
pub(super) fn package_friend_relative_path(
    path: &Path,
    package_root: &Path,
) -> Result<Option<PathBuf>, SkiffTestError> {
    match production_friend_match_for_test_file(path).map_err(|source| {
        SkiffTestError::ReadSource {
            path: path.display().to_string(),
            source,
        }
    })? {
        FriendProductionMatch::None => Ok(None),
        FriendProductionMatch::Unique(production_path) => Ok(Some(
            production_path
                .strip_prefix(package_root)
                .unwrap_or(&production_path)
                .to_path_buf(),
        )),
        FriendProductionMatch::Ambiguous(candidates) => {
            Err(ambiguous_friend_test_error(path, candidates))
        }
    }
}
pub(super) fn should_skip_package_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "target" || name == "node_modules" || name.starts_with('.')
}
pub(super) fn is_test_file_path(path: &Path) -> bool {
    skiff_compiler::test_support::is_test_file_path(path)
}
pub(super) fn find_package_root(input: &Path, input_is_file: bool) -> Option<PathBuf> {
    let mut current = if input_is_file {
        input.parent()?.to_path_buf()
    } else {
        input.to_path_buf()
    };
    loop {
        if current.join(PACKAGE_CONFIG_FILE).is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}
pub(super) fn find_service_root(input: &Path, input_is_file: bool) -> Option<PathBuf> {
    let mut current = if input_is_file {
        input.parent()?.to_path_buf()
    } else {
        input.to_path_buf()
    };
    loop {
        if current.join(SERVICE_CONFIG_FILE).is_file() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}
pub(super) fn read_root_sources(
    root: &Path,
    _profile: Option<&str>,
) -> Result<Vec<ParsedSource>, SkiffTestError> {
    let source_tree = collect_source_tree(root)?;
    let production_modules = source_tree
        .sources
        .iter()
        .filter(|source| !source.is_test_file)
        .map(|source| (source.file_path.clone(), source.module_path.clone()))
        .collect::<BTreeMap<_, _>>();
    source_tree
        .sources
        .iter()
        .map(|source| {
            let path = source_tree.root.join(&source.file_path);
            let text = fs::read_to_string(&path).map_err(|source| SkiffTestError::ReadSource {
                path: path.display().to_string(),
                source,
            })?;
            let ast = if source.is_test_file {
                parse_source(&text)
            } else {
                parse_source_with_bodies_tolerant(&text)
            }
            .map_err(|source| SkiffTestError::Parse {
                path: path.display().to_string(),
                source,
            })?;
            if !source.is_test_file {
                validate_no_test_declarations_in_production_source(
                    &path.display().to_string(),
                    &ast,
                )
                .map_err(|source| SkiffTestError::Parse {
                    path: path.display().to_string(),
                    source,
                })?;
            }
            let friend_module_path =
                if source.is_test_file {
                    let absolute_friend = match production_friend_match_for_test_file(&path)
                        .map_err(|source| SkiffTestError::ReadSource {
                            path: path.display().to_string(),
                            source,
                        })? {
                        FriendProductionMatch::None => None,
                        FriendProductionMatch::Unique(path) => Some(path),
                        FriendProductionMatch::Ambiguous(candidates) => {
                            return Err(ambiguous_friend_test_error(&path, candidates))
                        }
                    };
                    absolute_friend
                        .and_then(|path| {
                            path.strip_prefix(&source_tree.root)
                                .ok()
                                .map(Path::to_path_buf)
                        })
                        .and_then(|path| production_modules.get(&path).cloned())
                } else {
                    None
                };
            Ok(ParsedSource {
                source: source.clone(),
                text,
                ast,
                synthetic_imports: BTreeSet::new(),
                private_visibility_scope: friend_module_path
                    .clone()
                    .map(PrivateVisibilityScope::Module)
                    .unwrap_or_default(),
                friend_module_path,
            })
        })
        .collect()
}
pub(super) fn ambiguous_friend_test_error(path: &Path, candidates: Vec<PathBuf>) -> SkiffTestError {
    SkiffTestError::AmbiguousFriendTest {
        path: path.display().to_string(),
        candidates: candidates
            .iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

#[cfg(test)]
mod tests {
    use skiff_syntax::parser::parse_source;

    use super::package_test_ast_for_cases;

    #[test]
    fn package_test_ast_for_cases_generates_one_function_per_selected_test() {
        let ast = parse_source(
            r#"
                test "first" {
                    assert true
                }

                test "second" {
                    assert true
                }
            "#,
        )
        .expect("test source should parse");

        let generated =
            package_test_ast_for_cases(&ast, [(0, "__skiff_test_0"), (1, "__skiff_test_1")]);

        assert!(generated.tests.is_empty());
        assert!(generated.test_default_run.is_none());
        assert!(generated
            .functions
            .iter()
            .any(|function| function.name == "__skiff_test_0"));
        assert!(generated
            .functions
            .iter()
            .any(|function| function.name == "__skiff_test_1"));
    }
}
pub(super) fn collect_test_cases(
    sources: &[ParsedSource],
) -> Result<Vec<TestCase>, SkiffTestError> {
    let mut cases = Vec::new();
    let mut next_index = 0usize;
    for source in sources {
        cases.extend(test_cases_for_source(source, &mut next_index));
    }
    Ok(cases)
}
pub(super) fn test_cases_for_source(
    source: &ParsedSource,
    next_index: &mut usize,
) -> Vec<TestCase> {
    let mut cases = Vec::new();
    for (test_index, test) in source.ast.tests.iter().enumerate() {
        let function_name = format!("__skiff_test_{}", *next_index);
        *next_index += 1;
        cases.push(TestCase {
            module_path: source.source.module_path.clone(),
            name: test.name.clone(),
            test_index,
            source: source.clone(),
            function_name,
        });
    }
    cases
}
