use crate::{
    parsed_sources::ParsedCompilerSource,
    shared::ast::{source_text_without_test_declarations, SourceFile},
};

use super::entity::{
    AnchorModulePath, PublicationIdentity, SourceDeclarationAnchor, SourceDeclarationKind,
    SourceSelector, SymbolName,
};
use crate::shared::id::PublicationId;

/// 源码文本拼接 identity(角色 b:revision 变更检测历史入口)。
///
/// **注意**:这个字符串当前仍被 `PackageSourceModel` 保留以便向后兼容,但
/// `runtime_manifest.rs` 的 revision_id 计算已切换到 descriptor-based revision,
/// 不再直接使用它。见 P1b 任务说明。
pub fn source_identity(parsed_sources: &[ParsedCompilerSource]) -> String {
    let mut identity = String::new();
    for parsed in parsed_sources {
        let source = parsed.source();
        if source.is_test_file {
            continue;
        }
        let production_text = source_text_without_test_declarations(&source.text, parsed.ast());
        identity.push_str(&source.module_path);
        identity.push('\0');
        identity.push_str(&source.relative_path.display().to_string());
        identity.push('\0');
        identity.push_str(&production_text);
        identity.push('\0');
    }
    identity
}

/// 每个顶层 declaration 的结构化 anchor 集合(角色 a:declaration 身份源)。
///
/// anchor 由 `PublicationIdentity + SourceSelector(module_path + symbol + kind_hint) +
/// kind` 组成;**不含源码文本、不含 declaration 在文件/module 中的序号**,因此:
/// - 同一 module/file 内重排 declaration → anchor 不变;
/// - 加/删无关 sibling → 既有 anchor 不变;
/// - 把 declaration 移到不同 module/file → anchor 改变(即使 descriptor 和 public path 不变)。
///
/// `package_id` 必须是当前 package publication 的真实 ID；不能使用 placeholder/sentinel，
/// 否则不同 package 的同名 declaration 会得到相同 ABI owner identity。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationDeclarationAnchors {
    anchors: Vec<SourceDeclarationAnchor>,
}

impl PublicationDeclarationAnchors {
    pub fn build(parsed_sources: &[ParsedCompilerSource], package_id: &str) -> Self {
        let pub_id = PublicationId::parse(package_id)
            .expect("declaration anchor package_id must be a valid publication id");
        let publication = PublicationIdentity::with_default_epoch(pub_id);

        let mut anchors = Vec::new();
        for parsed in parsed_sources {
            let source = parsed.source();
            if source.is_test_file {
                continue;
            }
            let module_path = module_path_from_str(&source.module_path);
            anchors.extend(anchors_for_ast(&publication, &module_path, parsed.ast()));
        }
        Self { anchors }
    }

    // P2 will consume this to build AbiTypeId from each anchor.
    #[allow(dead_code)]
    pub fn anchors(&self) -> &[SourceDeclarationAnchor] {
        &self.anchors
    }
}

/// 从单个文件 AST 抽取其全部顶层声明的 anchor。
///
/// 这是 `build()` 与稳定性测试共享的单一路径:测试若想验"reorder/加 sibling 不改 anchor",
/// 必须走这条真实抽取逻辑,而不是另构造相同输入比相等(那只验 `derive(PartialEq)` 自反)。
///
/// `impl` method 不进入(架构 L309:不独立持有顶层 declaration anchor)。
fn anchors_for_ast(
    publication: &PublicationIdentity,
    module_path: &AnchorModulePath,
    ast: &SourceFile,
) -> Vec<SourceDeclarationAnchor> {
    let mut anchors = Vec::new();
    let mut push = |symbol: &str, kind: SourceDeclarationKind| {
        anchors.push(make_anchor(
            publication.clone(),
            module_path.clone(),
            symbol,
            kind,
        ));
    };
    for decl in &ast.types {
        push(&decl.name, SourceDeclarationKind::Type);
    }
    for decl in &ast.aliases {
        push(&decl.name, SourceDeclarationKind::Alias);
    }
    for decl in &ast.interfaces {
        push(&decl.name, SourceDeclarationKind::Interface);
    }
    // top-level functions only; impl methods are NOT independent top-level anchors.
    for decl in &ast.functions {
        push(&decl.name, SourceDeclarationKind::Function);
    }
    for decl in &ast.consts {
        push(&decl.name, SourceDeclarationKind::Const);
    }
    for decl in &ast.dbs {
        push(&decl.name, SourceDeclarationKind::DbObject);
    }
    anchors
}

fn module_path_from_str(module_path: &str) -> AnchorModulePath {
    let segments: Vec<String> = module_path.split('.').map(|s| s.to_string()).collect();
    AnchorModulePath::new(segments)
}

fn make_anchor(
    publication: PublicationIdentity,
    module_path: AnchorModulePath,
    symbol: &str,
    kind: SourceDeclarationKind,
) -> SourceDeclarationAnchor {
    let selector =
        SourceSelector::new(module_path, SymbolName::new(symbol.to_string()), Some(kind));
    SourceDeclarationAnchor::new(publication, selector, kind)
}

#[cfg(test)]
mod tests;
