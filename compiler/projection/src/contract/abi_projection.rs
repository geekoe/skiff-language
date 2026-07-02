//! Artifact projection:合约边界的 ABI 身份投影(T-P3a 派生层 + P3a-switch 已接入)。
//!
//! ## 范围状态
//!
//! 本模块提供"从 declaration anchor 派生 `AbiTypeId` / 产 `AbiIdentityProjection`"的派生层能力。
//!
//! **P3a-switch 已完成**:`abi_type_id_for_named_key` 现在被 `project.rs` 的真实身份判等路径
//! 消费——`ContractProjectionProjector::is_explicit_self_receiver`(project.rs:582 一带)的
//! named-vs-named 判等已从 `canonical_symbol` 字符串塌缩切到 `AbiTypeId` 判等(builtin/std/
//! external 等无 anchor 的情况保留结构化 fallback)。真实判等路径的 case #16/#20 测试见
//! `project.rs` 的 `explicit_self_receiver_uses_abi_type_id_identity`、
//! `same_shape_different_anchor_first_param_is_not_self_receiver`、
//! `self_receiver_detection_independent_of_public_path`(走 `project_contract_projection` 全
//! 管道,不是隔离派生函数)。
//!
//! 本模块下面的 case 测试是**派生层身份**的单元验证(直接对 `abi_type_id_for_source_key` 等
//! 派生函数断言),与真实判等路径测试互补:这里证派生正确,project.rs 那边证判等已消费派生结果。
//!
//! # 架构对应关系
//!
//! - 架构 L779-804 `Artifact Boundary` 节:export table 用 `AbiSymbolId`,
//!   closure table 区分 explicit public 与 closure-only。
//! - 架构 L532-533:public path 是 lookup/API binding,不是 type identity;
//!   type identity 是 `AbiTypeId`。
//! - 架构 L703-705:`ClosureOnly` type 有 `AbiTypeId` 但无 public path,不进 public exports。
//!
//! # 关键设计决策
//!
//! `ContractNamedTypeKey` 持有的是 `ProjectionSourceSymbolKey`(module_path + symbol),这是 file-local
//! canonical key。从它派生 `AbiTypeId` 需要 declaration anchor(包含 PublicationIdentity)。
//! 本模块通过 `ContractProjectionIndex` 的 `declaration_anchors()` 查表,按 module_path +
//! symbol 匹配,再通过 compiler 侧 `abi.rs` 的 `abi_type_id_from_anchor` 派生 `AbiTypeId`。
//!
//! 若 source_key 没有对应的 declaration anchor(例如 external/std symbol),则返回 `None`。
//! 这与架构 L694-702 中 `External` anchor 的处理分离:本 publication 的 source anchor 走
//! `AbiDeclarationAnchor::Source`,external 走 `External`。P3a 只处理本 publication 侧。
//!
//! # Verification 覆盖
//!
//! - case #16 same-shape≠equal:两个相同形状但不同 anchor 的 type → AbiTypeId 不同。
//! - case #17 two-public-paths 不合并:不同 source entity 的两个 public path → AbiTypeId 不同。
//! - case #20 changing public path 改 API binding:public path 是 lookup binding,AbiTypeId
//!   来自 anchor,改 public path 不改 AbiTypeId。
//! - case #21 closure-only 不混入 public exports:Source variant type 不进 public_symbols。
//! - case #22 service operation ref 不与 package export ref 合并:二者是不同 `ContractTypeKey`
//!   variant(`PackageSymbol` vs `Named`),不可能合并。

#![allow(dead_code)]

use std::collections::BTreeMap;

use skiff_artifact_model::{
    AbiIdentityFacts, AbiSymbolId, AbiSymbolIdFact, AbiTypeId, TypeNameability,
};

use skiff_compiler_projection_input::{ProjectionSourceDeclarationKind, ProjectionSourceSymbolKey};

use super::{model::ContractProjection, ContractNamedTypeKey, ContractProjectionIndex};

/// 合约边界 ABI 身份投影(架构 L791-804 `AbiIdentityProjection`)。
///
/// 这是 P3a 的核心产出。它与 `ContractProjection` 并行存在,后者保留用于 runtime schema
/// 序列化和 boundary validation,本结构专门提供 ABI 身份级别的视图。
///
/// 字段语义:
/// - `public_symbols`:public path → AbiSymbolId 投影(case #20/#21)。只包含
///   显式 public path 绑定的 symbol;closure-only type 不在其中(架构 L532-533)。
/// - `type_nameability`:AbiTypeId → TypeNameability。包含全部 named type
///   (public + closure-only),用于区分两类(case #21)。
///
/// 注:service operation ref(PackageSymbol)不在这个 map 里——它们不是 named source type
/// identity,而是 external package linkage(架构 L785,case #22)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiIdentityProjection {
    /// public_path → AbiSymbolId。只含 explicit public type/alias/interface,
    /// closure-only type 不进此 map(架构 L785,case #21)。
    pub public_symbols: BTreeMap<String, AbiSymbolId>,

    /// AbiTypeId → TypeNameability。包含 public + closure-only 全部 named source type。
    /// 消费方可用此 map 检查:同 AbiTypeId → 同 nominal type;不同 AbiTypeId → 不同 type
    /// (case #16)。
    pub type_nameability: BTreeMap<AbiTypeId, TypeNameability>,
}

impl AbiIdentityProjection {
    pub fn to_artifact_facts(&self) -> AbiIdentityFacts {
        AbiIdentityFacts {
            public_symbols: self
                .public_symbols
                .iter()
                .map(|(public_path, symbol)| (public_path.clone(), abi_symbol_id_to_fact(symbol)))
                .collect(),
            type_nameability: self
                .type_nameability
                .iter()
                .map(|(type_id, nameability)| (abi_id_key_hex(type_id.key_bytes()), *nameability))
                .collect(),
        }
    }
}

fn abi_symbol_id_to_fact(symbol: &AbiSymbolId) -> AbiSymbolIdFact {
    match symbol {
        AbiSymbolId::Type(id) => AbiSymbolIdFact::Type {
            abi_type_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Alias(id) => AbiSymbolIdFact::Alias {
            abi_alias_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Interface(id) => AbiSymbolIdFact::Interface {
            abi_interface_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Callable(id) => AbiSymbolIdFact::Callable {
            abi_callable_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Const(id) => AbiSymbolIdFact::Const {
            abi_const_id: abi_id_key_hex(id.key_bytes()),
        },
        AbiSymbolId::Instance(id) => AbiSymbolIdFact::Instance {
            abi_instance_id: abi_id_key_hex(id.key_bytes()),
        },
    }
}

fn abi_id_key_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// 从 `ContractProjection` 和 `ContractProjectionIndex` 产出 `AbiIdentityProjection`。
///
/// 本函数是 P3a 的"artifact projection 接 AbiTypeId"入口。
pub fn project_abi_identity(
    projection: &ContractProjection,
    index: &ContractProjectionIndex<'_>,
) -> AbiIdentityProjection {
    let mut public_symbols: BTreeMap<String, AbiSymbolId> = BTreeMap::new();
    let mut type_nameability: BTreeMap<AbiTypeId, TypeNameability> = BTreeMap::new();

    // ── Public types ──────────────────────────────────────────────────────────
    // types that are in public_symbols_by_source are PublicNameable;
    // types that are NOT are ClosureOnly (case #21).
    for (public_name, ty) in &projection.types {
        let source_key = ProjectionSourceSymbolKey::new(&ty.source_module, &ty.source_name);
        let is_public = projection
            .public_symbols_by_source
            .contains_key(&format!("{}.{}", ty.source_module, ty.source_name));

        if let Some(abi_type_id) =
            index.abi_type_id_for_source_key(&source_key, ProjectionSourceDeclarationKind::Type)
        {
            let nameability = if is_public {
                TypeNameability::PublicNameable
            } else {
                TypeNameability::ClosureOnly
            };
            type_nameability
                .entry(abi_type_id.clone())
                .or_insert(nameability);

            // Only public types go into the public_symbols export table (case #21).
            if is_public {
                public_symbols.insert(public_name.clone(), AbiSymbolId::Type(abi_type_id));
            }
        }
    }

    // ── Public aliases ────────────────────────────────────────────────────────
    // Aliases use AbiAliasId, not AbiTypeId (architecture L707-708).
    for (public_name, alias) in &projection.aliases {
        let source_key = ProjectionSourceSymbolKey::new(&alias.source_module, &alias.source_name);
        let is_public = projection
            .public_symbols_by_source
            .contains_key(&format!("{}.{}", alias.source_module, alias.source_name));

        if let Some(abi_alias_id) =
            index.abi_alias_id_for_source_key(&source_key, ProjectionSourceDeclarationKind::Alias)
        {
            // Aliases are PublicNameable when they have a public path (same logic as types).
            if is_public {
                public_symbols.insert(public_name.clone(), AbiSymbolId::Alias(abi_alias_id));
            }
        }
    }

    // ── Public interfaces ─────────────────────────────────────────────────────
    for (public_name, interface) in &projection.interfaces {
        let source_key =
            ProjectionSourceSymbolKey::new(&interface.source_module, &interface.source_name);
        let is_public = projection.public_symbols_by_source.contains_key(&format!(
            "{}.{}",
            interface.source_module, interface.source_name
        ));

        if let Some(abi_interface_id) = index.abi_interface_id_for_source_key(
            &source_key,
            ProjectionSourceDeclarationKind::Interface,
        ) {
            if is_public {
                public_symbols.insert(
                    public_name.clone(),
                    AbiSymbolId::Interface(abi_interface_id),
                );
            }
        }
    }

    AbiIdentityProjection {
        public_symbols,
        type_nameability,
    }
}

/// `ContractNamedTypeKey` ABI identity 查询:给定 index 上下文,返回该 named type 的 `AbiTypeId`。
///
/// 返回值:
/// - `Some(id)` — 找到对应 declaration anchor,派生出 AbiTypeId。
/// - `None` — 无对应 anchor(例如 external/std symbol)。
pub fn abi_type_id_for_named_key(
    key: &ContractNamedTypeKey,
    index: &ContractProjectionIndex<'_>,
) -> Option<AbiTypeId> {
    match key {
        ContractNamedTypeKey::Public { symbol } => {
            // 通过 public path 反查 source key,再从 anchor 派生 AbiTypeId。
            // case #17:两个不同 source entity 不能因 public path 相同而得到相同 AbiTypeId——
            // 因为 AbiTypeId 来自 anchor(module_path + symbol),不来自 public path 字符串。
            let source_key = index.source_key_for_named_key_public(symbol)?;
            index.abi_type_id_for_source_key(&source_key, ProjectionSourceDeclarationKind::Type)
        }
        ContractNamedTypeKey::Source { source } => {
            // closure-only type:直接从 ProjectionSourceSymbolKey 派生。
            index.abi_type_id_for_source_key(source, ProjectionSourceDeclarationKind::Type)
        }
    }
}
