//! Compiler 侧:从 `SourceDeclarationAnchor` 派生 ABI nominal id(T-P2a)。
//!
//! 本模块是 compiler 与 `artifact-model` ABI 类型层的桥接。它消费 compiler 内部的
//! `SourceDeclarationAnchor` 类型,产出 `artifact-model` 的 `AbiTypeId` 等类型。
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
//! # 跨 artifact canonical encoding(TODO,后续阶段)
//!
//! P2a 阶段的 key 编码是结构化 null-byte 分隔字节序列,满足"同 → 同,异 → 异"但尚未做
//! sha256。跨 artifact 的稳定 canonical bytes 留给 `artifact-identity` crate(后续)。
//!
//! # impl method ABI 身份(保守落法,架构 L309)
//!
//! impl method 不独立持有顶层 declaration anchor。若需在此层表示 impl method,使用
//! `(owner AbiTypeId, method_name)` 组合,不调用本模块的顶层派生函数。
//! **这是按架构 L309 的保守落法。**

#![allow(dead_code)]

use skiff_artifact_model::abi_identity::derive::{
    abi_alias_id_from_source_anchor, abi_callable_id_from_source_anchor,
    abi_const_id_from_source_anchor, abi_instance_id_from_source_anchor,
    abi_interface_id_from_source_anchor, abi_type_id_from_source_anchor, AbiSourceAnchorInput,
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
            .map(|s| s.clone())
            .collect(),
        symbol: anchor.selector().symbol().as_str().to_string(),
        kind: project_kind(anchor.kind()),
    }
}

/// `SourceDeclarationAnchor` 转 `AbiSourceAnchorInput`(供 derive 函数消费)。
fn to_anchor_input(anchor: &SourceDeclarationAnchor) -> AbiSourceAnchorInput {
    AbiSourceAnchorInput {
        publication_id: anchor.publication().id().as_str().to_string(),
        abi_epoch: anchor.publication().abi_epoch().value(),
        module_path: anchor
            .selector()
            .module_path()
            .segments()
            .iter()
            .map(|s| s.clone())
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
    let input = to_anchor_input(anchor);
    abi_type_id_from_source_anchor(&input, type_args)
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiAliasId`。
///
/// Alias 不建 `AbiTypeId`;按 target 展开(架构 L707-708)。
pub fn abi_alias_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiAliasId {
    let input = to_anchor_input(anchor);
    abi_alias_id_from_source_anchor(&input)
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiInterfaceId`。
pub fn abi_interface_id_from_anchor(
    anchor: &SourceDeclarationAnchor,
    type_args: &[AbiTypeId],
) -> AbiInterfaceId {
    let input = to_anchor_input(anchor);
    abi_interface_id_from_source_anchor(&input, type_args)
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiCallableId`。
pub fn abi_callable_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiCallableId {
    let input = to_anchor_input(anchor);
    abi_callable_id_from_source_anchor(&input)
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiConstId`。
pub fn abi_const_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiConstId {
    let input = to_anchor_input(anchor);
    abi_const_id_from_source_anchor(&input)
}

/// 从 `SourceDeclarationAnchor` 派生 `AbiInstanceId`。
pub fn abi_instance_id_from_anchor(anchor: &SourceDeclarationAnchor) -> AbiInstanceId {
    let input = to_anchor_input(anchor);
    abi_instance_id_from_source_anchor(&input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{
        AnchorModulePath, PublicationIdentity, SourceDeclarationAnchor, SourceDeclarationKind,
        SourceSelector, SymbolName,
    };
    use crate::shared::id::PublicationId;

    fn make_anchor(
        pub_id: &str,
        module_segs: &[&str],
        symbol: &str,
        kind: SourceDeclarationKind,
    ) -> SourceDeclarationAnchor {
        let pub_id = PublicationId::parse(pub_id).expect("valid publication id");
        let publication = PublicationIdentity::with_default_epoch(pub_id);
        let module_path =
            AnchorModulePath::new(module_segs.iter().map(|s| s.to_string()).collect());
        let selector =
            SourceSelector::new(module_path, SymbolName::new(symbol.to_string()), Some(kind));
        SourceDeclarationAnchor::new(publication, selector, kind)
    }

    /// 架构 Verification #16: same-shape ≠ equal(compiler 侧)。
    ///
    /// 两个不同 module path 的 anchor 派生出不同 AbiTypeId。
    #[test]
    fn same_shape_not_equal_compiler_side() {
        let anchor_a = make_anchor(
            "example.com/pkg",
            &["foo", "bar"],
            "MyType",
            SourceDeclarationKind::Type,
        );
        let anchor_b = make_anchor(
            "example.com/pkg",
            &["foo", "baz"],
            "MyType",
            SourceDeclarationKind::Type,
        );

        let id_a = abi_type_id_from_anchor(&anchor_a, &[]);
        let id_b = abi_type_id_from_anchor(&anchor_b, &[]);

        assert_ne!(
            id_a, id_b,
            "不同 module path 的 anchor 应派生出不同 AbiTypeId"
        );
    }

    /// 架构 Verification #12: move-selector 改 nominal(compiler 侧)。
    #[test]
    fn move_selector_changes_nominal_compiler_side() {
        let before = make_anchor(
            "example.com/pkg",
            &["original", "module"],
            "User",
            SourceDeclarationKind::Type,
        );
        let after = make_anchor(
            "example.com/pkg",
            &["moved", "module"],
            "User",
            SourceDeclarationKind::Type,
        );

        assert_ne!(
            abi_type_id_from_anchor(&before, &[]),
            abi_type_id_from_anchor(&after, &[]),
            "symbol 移到不同 module 后 AbiTypeId 应改变"
        );
    }

    /// 架构 Verification #13: descriptor 改不影响 nominal(compiler 侧)。
    ///
    /// 同一 anchor 两次调用派生相同 AbiTypeId。
    #[test]
    fn same_anchor_same_nominal_id() {
        let anchor = make_anchor(
            "example.com/pkg",
            &["api"],
            "Order",
            SourceDeclarationKind::Type,
        );

        let id1 = abi_type_id_from_anchor(&anchor, &[]);
        let id2 = abi_type_id_from_anchor(&anchor, &[]);

        assert_eq!(id1, id2, "同一 anchor 派生的 AbiTypeId 应相等");
    }

    /// 泛型实参进 nominal(compiler 侧)。
    #[test]
    fn type_args_affect_nominal_id_compiler_side() {
        let container_anchor = make_anchor(
            "example.com/pkg",
            &["collections"],
            "List",
            SourceDeclarationKind::Type,
        );
        let int_anchor = make_anchor(
            "skiff.run/std",
            &["primitives"],
            "Int",
            SourceDeclarationKind::Type,
        );
        let str_anchor = make_anchor(
            "skiff.run/std",
            &["primitives"],
            "String",
            SourceDeclarationKind::Type,
        );

        let id_int = abi_type_id_from_anchor(&int_anchor, &[]);
        let id_str = abi_type_id_from_anchor(&str_anchor, &[]);

        let list_int = abi_type_id_from_anchor(&container_anchor, &[id_int]);
        let list_str = abi_type_id_from_anchor(&container_anchor, &[id_str]);

        assert_ne!(list_int, list_str, "不同 type args 应产出不同 AbiTypeId");
    }

    /// Alias 走 AbiAliasId,不走 AbiTypeId(compiler 侧)。
    #[test]
    fn alias_uses_abi_alias_id_not_type_id_compiler_side() {
        let anchor = make_anchor(
            "example.com/pkg",
            &["api"],
            "UserId",
            SourceDeclarationKind::Alias,
        );

        // 编译时已保证类型区分:abi_alias_id_from_anchor 返回 AbiAliasId 而非 AbiTypeId
        let alias_id = abi_alias_id_from_anchor(&anchor);

        // alias_id 是 AbiAliasId,不能放入 AbiSymbolId::Type
        // 用 Alias variant 包装验证类型
        use skiff_artifact_model::AbiSymbolId;
        let sym = AbiSymbolId::Alias(alias_id);
        assert!(matches!(sym, AbiSymbolId::Alias(_)));
    }

    /// project_source_anchor 完整性:字段正确投影。
    #[test]
    fn project_source_anchor_fields() {
        let anchor = make_anchor(
            "example.com/pkg",
            &["foo", "bar"],
            "MySymbol",
            SourceDeclarationKind::Type,
        );

        let projected = project_source_anchor(&anchor);

        assert_eq!(projected.publication_id, "example.com/pkg");
        assert_eq!(projected.abi_epoch, 0);
        assert_eq!(projected.module_path, vec!["foo", "bar"]);
        assert_eq!(projected.symbol, "MySymbol");
        assert_eq!(projected.kind, AbiDeclarationKind::Type);
    }

    /// 架构 Verification case #19:interface identity 包含 symbol + 完整 type args,
    /// 不伪装成 record。
    ///
    /// 验证点:
    /// 1. interface anchor 派生 `AbiInterfaceId`,不派生 `AbiTypeId`(不伪装成 record/type)。
    /// 2. 同一 interface symbol 但不同 type args → 不同 `AbiInterfaceId`(type args 是 identity 的一部分)。
    /// 3. `AbiInterfaceId` 与 `AbiTypeId` 是不同 Rust 类型,Rust 类型系统在编译期已阻止混用。
    #[test]
    fn case19_interface_identity_contains_symbol_and_full_type_args_not_record() {
        let sink_anchor = make_anchor(
            "example.com/pkg",
            &["api"],
            "EventSink",
            SourceDeclarationKind::Interface,
        );

        // No type args: AbiInterfaceId derived from anchor alone.
        let id_bare = abi_interface_id_from_anchor(&sink_anchor, &[]);

        // With one type arg: same interface symbol but different instantiation.
        let type_anchor = make_anchor(
            "example.com/pkg",
            &["api"],
            "Event",
            SourceDeclarationKind::Type,
        );
        let event_abi_type_id = abi_type_id_from_anchor(&type_anchor, &[]);
        let id_with_event =
            abi_interface_id_from_anchor(&sink_anchor, &[event_abi_type_id.clone()]);

        // Different type args → different AbiInterfaceId.
        assert_ne!(
            id_bare, id_with_event,
            "case #19: same interface symbol with different type args must have different AbiInterfaceId"
        );

        // AbiInterfaceId is distinct from AbiTypeId — interface does not masquerade as a record type.
        // Rust type system enforces this: AbiInterfaceId and AbiTypeId are different newtypes.
        let as_symbol_interface = skiff_artifact_model::AbiSymbolId::Interface(id_bare);
        let as_symbol_type =
            skiff_artifact_model::AbiSymbolId::Type(abi_type_id_from_anchor(&sink_anchor, &[]));
        assert_ne!(
            as_symbol_interface, as_symbol_type,
            "case #19: interface must not be represented as a type (AbiSymbolId::Interface ≠ AbiSymbolId::Type)"
        );
        assert!(
            matches!(
                as_symbol_interface,
                skiff_artifact_model::AbiSymbolId::Interface(_)
            ),
            "case #19: interface identity must use AbiSymbolId::Interface variant, not Type"
        );

        // Cross-check: two different interface anchors have different ids.
        let stream_anchor = make_anchor(
            "example.com/pkg",
            &["api"],
            "Stream",
            SourceDeclarationKind::Interface,
        );
        let id_stream = abi_interface_id_from_anchor(&stream_anchor, &[event_abi_type_id]);
        assert_ne!(
            id_with_event, id_stream,
            "different interface symbols with same type arg must have different AbiInterfaceId"
        );
    }

    /// 不同 publication id → 不同 AbiTypeId。
    #[test]
    fn different_publication_different_id() {
        let anchor_a = make_anchor(
            "example.com/pkg-a",
            &["api"],
            "Foo",
            SourceDeclarationKind::Type,
        );
        let anchor_b = make_anchor(
            "example.com/pkg-b",
            &["api"],
            "Foo",
            SourceDeclarationKind::Type,
        );

        assert_ne!(
            abi_type_id_from_anchor(&anchor_a, &[]),
            abi_type_id_from_anchor(&anchor_b, &[]),
            "不同 publication 的同名 symbol 应有不同 AbiTypeId"
        );
    }
}
