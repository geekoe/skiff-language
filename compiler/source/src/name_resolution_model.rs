use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use crate::{
    entity::{
        resolve_dotted_path, EntityId, EntityNamespace, ExternalPackageEntityId,
        PublicationEntityTable, ResolutionEnv, ResolvedPath,
    },
    parsed_sources::ParsedCompilerSource,
    shared::publication_error::PublicationError,
    source_graph::CompilerSourceFile,
    source_name_resolution::{
        build_source_name_resolution_file_facts,
        collect_unresolved_dotted_root_violations_from_facts, SourceNameResolutionFileBuildInput,
        SourceNameResolutionFileFacts,
    },
};

/// Source-level name resolution model。
///
/// 这是 `SourceCompileModel` 的 `name_resolution` 字段——pipeline 文档要求的"单一事实来源"。
/// 它捕获了 source compile 阶段完成 name resolution 所需的所有信息,让 lowering 直接消费
/// 而不必重算。
///
/// 形状设计:
/// - `module_roots`:当前 publication 内所有 production source 文件的 module path 第一段
///   (如 `user`、`billing`)。诊断 pass 和 lowering 都需要这些来判断"module 前缀"。
/// - `source_files`:每个 production source 的 value/type namespace facts 以及 dotted path
///   `ResolvedPath` 结果入口。诊断 pass 从这里消费结果,不再遍历 AST 自算可见 root。
/// - `package_aliases_map`:package 依赖别名到 path 分段的完整映射(供 lowering 做
///   `PackageExportResolver` 路径解析)。这是兼容 shim;等 file_ir lowering / PackageExportResolver
///   接受 `ExternalPackageEntityId` keyed facts 后删除。
/// - `package_alias_entities`:package 依赖别名到 typed `ExternalPackageEntityId` 的映射。
/// - `package_aliases_set`:package 依赖别名名字集(供诊断 pass 和 resolve_dotted_path)。
/// - `service_aliases`:service 依赖别名集(来自 service.yml dependencies)。
#[derive(Debug, Clone)]
pub struct NameResolutionModel {
    /// 当前 publication 内 production source 的 module root 集(如 `user`、`billing`)。
    module_roots: BTreeSet<String>,
    /// production source file -> namespace facts + per-path typed resolution results。
    source_files: BTreeMap<String, SourceNameResolutionFileFacts>,
    /// package 依赖别名 -> path 分段(供 lowering PackageExportResolver 消费)。
    package_aliases_map: BTreeMap<String, Vec<String>>,
    /// package 依赖别名 -> typed external package entity id。
    package_alias_entities: BTreeMap<String, ExternalPackageEntityId>,
    /// package 依赖别名名字集(供 resolve_dotted_path / 诊断 pass 消费)。
    package_aliases_set: BTreeSet<String>,
    /// service 依赖别名集(来自 service.yml dependencies alias)。
    service_aliases: BTreeSet<String>,
}

impl NameResolutionModel {
    /// 从已解析的 sources 和依赖别名构造 model。
    #[cfg(test)]
    pub fn build(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        service_dependency_aliases: &BTreeSet<String>,
    ) -> Self {
        Self::build_with(
            parsed_sources,
            package_aliases,
            service_dependency_aliases,
            None,
        )
    }

    pub fn build_with(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        service_dependency_aliases: &BTreeSet<String>,
        current_publication_entities: Option<&PublicationEntityTable>,
    ) -> Self {
        let module_roots = parsed_sources
            .iter()
            .filter(|parsed| !parsed.source().is_test_file)
            .filter_map(|parsed| parsed.source().module_path.split('.').next())
            .filter(|root| !root.is_empty())
            .map(str::to_string)
            .collect();
        let package_aliases_set = package_aliases.keys().cloned().collect();
        let package_aliases_map = package_aliases.clone();
        let package_alias_entities = typed_package_alias_entities(&package_aliases_set);
        let service_aliases = service_dependency_aliases.clone();
        let additional_value_roots = BTreeSet::new();
        let source_files = parsed_sources
            .iter()
            .filter(|parsed| !parsed.source().is_test_file)
            .map(|parsed| {
                let relative_path = parsed.source().relative_path.display().to_string();
                let facts =
                    build_source_name_resolution_file_facts(SourceNameResolutionFileBuildInput {
                        relative_path: &relative_path,
                        module_path: &parsed.source().module_path,
                        ast: parsed.ast(),
                        module_roots: &module_roots,
                        package_aliases: &package_aliases_set,
                        service_aliases: &service_aliases,
                        additional_value_roots: &additional_value_roots,
                        current_publication_entities,
                    });
                (relative_path, facts)
            })
            .collect();
        Self {
            module_roots,
            source_files,
            package_aliases_map,
            package_alias_entities,
            package_aliases_set,
            service_aliases,
        }
    }

    pub fn build_from_compiler_sources(
        sources: &[CompilerSourceFile],
        package_aliases: &BTreeMap<String, Vec<String>>,
        service_dependency_aliases: &BTreeSet<String>,
    ) -> Self {
        let additional_value_roots = BTreeSet::new();
        Self::build_from_compiler_sources_with_additional_value_roots(
            sources,
            package_aliases,
            service_dependency_aliases,
            &additional_value_roots,
        )
    }

    pub fn build_from_compiler_sources_with_additional_value_roots(
        sources: &[CompilerSourceFile],
        package_aliases: &BTreeMap<String, Vec<String>>,
        service_dependency_aliases: &BTreeSet<String>,
        additional_value_roots: &BTreeSet<String>,
    ) -> Self {
        let module_roots = sources
            .iter()
            .filter(|source| !source.is_test_file)
            .filter_map(|source| source.module_path.split('.').next())
            .filter(|root| !root.is_empty())
            .map(str::to_string)
            .collect();
        let package_aliases_set = package_aliases.keys().cloned().collect();
        let package_aliases_map = package_aliases.clone();
        let package_alias_entities = typed_package_alias_entities(&package_aliases_set);
        let service_aliases = service_dependency_aliases.clone();
        let source_files = sources
            .iter()
            .filter(|source| !source.is_test_file)
            .map(|source| {
                let relative_path = source.relative_path.display().to_string();
                let facts =
                    build_source_name_resolution_file_facts(SourceNameResolutionFileBuildInput {
                        relative_path: &relative_path,
                        module_path: &source.module_path,
                        ast: &source.ast,
                        module_roots: &module_roots,
                        package_aliases: &package_aliases_set,
                        service_aliases: &service_aliases,
                        additional_value_roots,
                        current_publication_entities: None,
                    });
                (relative_path, facts)
            })
            .collect();
        Self {
            module_roots,
            source_files,
            package_aliases_map,
            package_alias_entities,
            package_aliases_set,
            service_aliases,
        }
    }

    /// 返回 package 别名名字集(供 resolve_dotted_path / 诊断 pass)。
    pub fn package_alias_names(&self) -> &BTreeSet<String> {
        &self.package_aliases_set
    }

    /// 返回 package 别名 -> path 分段的完整映射(供 lowering PackageExportResolver)。
    pub fn package_aliases_map(&self) -> &BTreeMap<String, Vec<String>> {
        &self.package_aliases_map
    }

    /// 返回 package 别名 -> typed `ExternalPackageEntityId` 映射。
    pub fn package_alias_entities(&self) -> &BTreeMap<String, ExternalPackageEntityId> {
        &self.package_alias_entities
    }

    /// 返回 service 别名集的引用(供 lowering 和诊断 pass)。
    pub fn service_aliases(&self) -> &BTreeSet<String> {
        &self.service_aliases
    }

    /// 返回 module root 集的引用(供诊断和 lowering 消费)。
    pub fn module_roots(&self) -> &BTreeSet<String> {
        &self.module_roots
    }

    pub fn source_file_facts_by_relative_path(
        &self,
        relative_path: &Path,
    ) -> Option<&SourceNameResolutionFileFacts> {
        self.source_files.get(&relative_path.display().to_string())
    }
}

fn typed_package_alias_entities(
    package_aliases: &BTreeSet<String>,
) -> BTreeMap<String, ExternalPackageEntityId> {
    let empty = BTreeSet::new();
    let env = ResolutionEnv::new(&empty, package_aliases, &empty, &empty, &empty, None, None);
    package_aliases
        .iter()
        .filter_map(
            |alias| match resolve_dotted_path(&env, alias, EntityNamespace::Value) {
                Some(ResolvedPath::Entity(EntityId::ExternalPackage(id))) => {
                    Some((alias.clone(), id))
                }
                _ => None,
            },
        )
        .collect()
}

pub fn validate_source_name_resolution_from_model(
    diagnostic_root: &Path,
    parsed_sources: &[ParsedCompilerSource],
    name_resolution: &NameResolutionModel,
) -> Result<(), PublicationError> {
    let mut violations = Vec::new();

    for parsed in parsed_sources
        .iter()
        .filter(|parsed| !parsed.source().is_test_file)
    {
        let path = diagnostic_root
            .join(&parsed.source().relative_path)
            .display()
            .to_string();
        let facts = name_resolution
            .source_file_facts_by_relative_path(&parsed.source().relative_path)
            .expect("name_resolution must include every production source file");
        collect_unresolved_dotted_root_violations_from_facts(&path, facts, &mut violations);
    }

    if violations.is_empty() {
        return Ok(());
    }

    Err(PublicationError::ContractValidation {
        message: format!(
            "source name resolution failed:\n{}",
            violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    })
}
