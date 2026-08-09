//! Compiler 侧:从 `SourceDeclarationAnchor` 派生 ABI nominal id(T-P2a)。
//!
//! 本模块是 compiler source anchor 到 canonical artifact identity API 的 typed adapter。
//!
//! # 依赖方向
//!
//! compiler → artifact-model(单向)。artifact-model 不依赖 compiler,因此
//! `AbiDeclarationAnchor::Source` 持有的是 artifact-model 侧自己定义的
//! `AbiSourceDeclarationAnchor`,不是 compiler 的 `SourceDeclarationAnchor`。
//! 本模块的 `project_source_anchor` 函数负责完成这个投影。
//!
//! # 语义约束(架构 L611-620)
//!
//! - 派生只吃 anchor + type args——不吞 descriptor bytes / schema hash / publication
//!   version / build id。
//! - 同 anchor + 同 type_args → 同 id;任何字段变 → id 变。
//!
//! # impl method ABI 身份(保守落法,架构 L309)
//!
//! impl method 不独立持有顶层 declaration anchor。若需在此层表示 impl method,使用
//! `(owner AbiTypeId, method_name)` 组合,不调用本模块的顶层派生函数。
//! **这是按架构 L309 的保守落法。**

#![allow(dead_code)]

use skiff_artifact_identity::{
    abi_alias_id_from_source_anchor, abi_callable_id_from_source_anchor,
    abi_const_id_from_source_anchor, abi_instance_id_from_source_anchor,
    abi_interface_id_from_source_anchor, abi_type_id_from_source_anchor,
};
use skiff_artifact_model::{
    AbiAliasId, AbiCallableId, AbiConstId, AbiDeclarationKind, AbiInstanceId, AbiInterfaceId,
    AbiSourceDeclarationAnchor, AbiTypeId,
};

use super::{SourceDeclarationAnchor, SourceDeclarationKind};

/// Compiler 的 `SourceDeclarationKind` 投影到 artifact-model 的 `AbiDeclarationKind`。
fn project_kind(kind: SourceDeclarationKind) -> AbiDeclarationKind {
    match kind {
        SourceDeclarationKind::Type => AbiDeclarationKind::Type,
        SourceDeclarationKind::Alias => AbiDeclarationKind::Alias,
        SourceDeclarationKind::Interface => AbiDeclarationKind::Interface,
        SourceDeclarationKind::Function => AbiDeclarationKind::Callable,
        SourceDeclarationKind::Const => AbiDeclarationKind::Const,
        SourceDeclarationKind::DbObject => AbiDeclarationKind::Instance,
    }
}

/// Compiler 的 `SourceDeclarationAnchor` 投影到 artifact-model 的
/// `AbiSourceDeclarationAnchor`(纯数据,用于 `AbiDeclarationAnchor::Source`)。
pub fn project_source_anchor(anchor: &SourceDeclarationAnchor) -> AbiSourceDeclarationAnchor {
    AbiSourceDeclarationAnchor {
        publication_id: anchor.publication().id().as_str().to_string(),
        abi_epoch: anchor.publication().abi_epoch().value(),
        module_path: anchor
            .selector()
            .module_path()
            .segments()
            .iter()
            .cloned()
            .collect(),
        symbol: anchor.selector().symbol().as_str().to_string(),
        kind: project_kind(anchor.kind()),
    }
}

/// 从 `SourceDeclarationAnchor` + type args 派生 `AbiTypeId`。
///
/// # 语义约束
///
/// 派生输入只有 anchor(publication identity + module path + symbol + kind)和 type_args;
/// 不吞 descriptor / schema hash / publication version / build id(架构 L618-620)。
///
/// 同一 anchor + 同 type_args → 相同 AbiTypeId。
/// anchor 任何字段变(module path 或 symbol)→ 不同 AbiTypeId。
/// type_args 变 → 不同 AbiTypeId。
pub fn abi_type_id_from_anchor(
    anchor: &SourceDeclarationAnchor,
    type_args: &[AbiTypeId],
) -> AbiTypeId {
    abi_type_id_from_source_anchor(&project_source_anchor(anchor), type_args)
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiAliasId`。
///
/// Alias 不建 `AbiTypeId`;按 target 展开(架构 L707-708)。
pub fn abi_alias_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiAliasId {
    abi_alias_id_from_source_anchor(&project_source_anchor(anchor))
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiInterfaceId`。
pub fn abi_interface_id_from_anchor(
    anchor: &SourceDeclarationAnchor,
    type_args: &[AbiTypeId],
) -> AbiInterfaceId {
    abi_interface_id_from_source_anchor(&project_source_anchor(anchor), type_args)
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiCallableId`。
pub fn abi_callable_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiCallableId {
    abi_callable_id_from_source_anchor(&project_source_anchor(anchor))
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiConstId`。
pub fn abi_const_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiConstId {
    abi_const_id_from_source_anchor(&project_source_anchor(anchor))
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiInstanceId`。
pub fn abi_instance_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiInstanceId {
    abi_instance_id_from_source_anchor(&project_source_anchor(anchor))
}

#[cfg(test)]
mod tests;
