//! ABI nominal identity 类型层(架构 `ABI Nominal Identity And Contract Revision` L595-637、
//! `ABI Type Identity` L639-712)。
//!
//! 这个模块由 T-P2a 任务建立。它只定义纯数据类型(+ 一个结构化派生辅助),**不实现**跨
//! artifact 稳定哈希——canonical bytes 计算留给 `artifact-identity` crate(后续阶段)。
//!
//! # 依赖方向约束
//!
//! 本 crate(`artifact-model`)不依赖 compiler crate,不能反向依赖。因此
//! `AbiDeclarationAnchor::Source` variant 持有的是 `AbiSourceDeclarationAnchor`(本模块自己
//! 定义的 ABI 侧纯数据表示),不是 compiler 的 `SourceDeclarationAnchor`(那在 compiler crate
//! 里,引它会成环)。compiler 侧的 `abi.rs` 负责把 compiler 的 anchor 投影到这里的类型。
//!
//! # 关键语义约束(架构 L611-620)
//!
//! - `AbiTypeId` 的语义输入**只**包含:owning publication identity + declaration anchor +
//!   symbol kind + 泛型实例化时完整 type arguments 的 `AbiTypeId`。
//! - `AbiTypeId` **不吞入** descriptor bytes、schema hash、publication version 或 build id。
//! - `AbiContractRevision` **不自带** `AbiSymbolId`;它的归属由外层 fact 决定(架构 L631-633)。
//! - nominal(anchor 派生)与 contract revision(descriptor hash)分离:descriptor 改 → revision
//!   变、nominal 不变;anchor 改 → nominal 变、descriptor 同也算不同 nominal(架构 L561-564)。
//!
//! # impl method ABI 身份(保守落法,架构 L309)
//!
//! impl method **不**独立持有顶层 declaration anchor,其 ABI 身份由 owning type/interface 的
//! anchor + method name 在 owner 的 descriptor 内承载。P2a 不给 impl method 造独立
//! `AbiTypeId`/anchor;若需表示,用 `(owner AbiTypeId/anchor, method_name)` 组合。
//! **这是按架构 L309 的保守落法,不创建 impl method 独立顶层 nominal anchor。**
//!
//! # `PublishedDeclarationId.stable_id` 生成机制(存根,待定)
//!
//! `stable_id` 是 dependency artifact 发布时固化的不透明 token(架构 L699)。P2a **只建类型
//! 存根**,不实现生成逻辑:stable_id 的生成牵扯发布 / registry 侧,机制尚未定义。本
//! publication 的 source anchor 走 `AbiDeclarationAnchor::Source`,不需要 `stable_id`;它只
//! 用于 `External` 跨 artifact 引用。

#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// ABI 所有 symbol 种类的 nominal id 联合(架构 L600-609)。
///
/// 注意:`Alias` 有 `AbiAliasId` 但**不建 nominal `AbiTypeId`**;alias 按 target 展开(架构
/// L707-708)。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AbiSymbolId {
    Type(AbiTypeId),
    Alias(AbiAliasId),
    Interface(AbiInterfaceId),
    Callable(AbiCallableId),
    Const(AbiConstId),
    Instance(AbiInstanceId),
}

/// 判断"是否同一类型"的 canonical key(架构 L639-641)。
///
/// # 语义输入(架构 L611-616)
///
/// `AbiTypeId` 的内容由以下字段结构化确定:
/// - owning publication id(string,stable)
/// - abi epoch
/// - declaration anchor(module path 分段 + symbol + kind)
/// - symbol kind(type/interface/etc.)
/// - 泛型实参的完整 `AbiTypeId` 列表
///
/// **不包含** descriptor bytes、schema hash、publication version 或 build id。
///
/// # 派生机制(P2a 阶段)
///
/// P2a 阶段用结构化字段拼合保证"同 anchor+args → 同 id、异 → 异"语义:字段 `key` 是把上述
/// 语义输入逐字段 null-byte 分隔后编码的稳定字节序列。跨 artifact 的 canonical sha256 encoding
/// 留给 `artifact-identity` crate(后续阶段)。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiTypeId {
    /// 结构化 stable key:由 publication id、abi epoch、module path 段、symbol、kind 和
    /// type args 的 AbiTypeId key 按确定性顺序拼接而成。
    ///
    /// P2a 使用 null-byte 分隔的字节向量;后续 artifact-identity 层会替换为 sha256。
    key: Vec<u8>,
}

impl AbiTypeId {
    /// 内部构造,仅供 compiler 侧派生函数和测试使用。
    pub fn from_key_bytes(key: Vec<u8>) -> Self {
        Self { key }
    }

    /// 仅供测试 / 调试:暴露内部 key bytes。
    pub fn key_bytes(&self) -> &[u8] {
        &self.key
    }
}

/// Alias 的 nominal id(架构 L707)。Alias 不建 `AbiTypeId`;按 target 展开。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiAliasId {
    key: Vec<u8>,
}

impl AbiAliasId {
    pub fn from_key_bytes(key: Vec<u8>) -> Self {
        Self { key }
    }

    pub fn key_bytes(&self) -> &[u8] {
        &self.key
    }
}

/// Interface 的 nominal id(架构 L710)。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiInterfaceId {
    key: Vec<u8>,
}

impl AbiInterfaceId {
    pub fn from_key_bytes(key: Vec<u8>) -> Self {
        Self { key }
    }

    pub fn key_bytes(&self) -> &[u8] {
        &self.key
    }
}

/// Callable 的 nominal id。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiCallableId {
    key: Vec<u8>,
}

impl AbiCallableId {
    pub fn from_key_bytes(key: Vec<u8>) -> Self {
        Self { key }
    }

    pub fn key_bytes(&self) -> &[u8] {
        &self.key
    }
}

/// Const 的 nominal id。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiConstId {
    key: Vec<u8>,
}

impl AbiConstId {
    pub fn from_key_bytes(key: Vec<u8>) -> Self {
        Self { key }
    }

    pub fn key_bytes(&self) -> &[u8] {
        &self.key
    }
}

/// Instance 的 nominal id。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiInstanceId {
    key: Vec<u8>,
}

impl AbiInstanceId {
    pub fn from_key_bytes(key: Vec<u8>) -> Self {
        Self { key }
    }

    pub fn key_bytes(&self) -> &[u8] {
        &self.key
    }
}

/// Descriptor / schema / signature 变化的 revision(架构 L622-633)。
///
/// **不自带 `AbiSymbolId`**:归属由外层 fact 决定,避免 nominal id 与 contract revision 双存
/// 不一致(架构 L631-633)。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AbiContractRevision {
    pub descriptor_hash: DescriptorHash,
    pub schema_revision: SchemaRevision,
}

/// descriptor 的哈希(P2a 阶段为不透明字节;实际 sha256 由 artifact-identity 层产)。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DescriptorHash(pub Vec<u8>);

/// schema / wire 兼容性 revision(P2a 为存根;实际语义由 P3a/P4 定义)。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SchemaRevision(pub u32);

/// Declaration anchor 的 ABI 侧联合(架构 L661-667)。
///
/// `Source` variant 持有 `AbiSourceDeclarationAnchor`(本模块自己定义的纯数据投影),不持有
/// compiler 的 `SourceDeclarationAnchor`(避免反向依赖 compiler crate)。Compiler 侧派生函数
/// 负责从 `SourceDeclarationAnchor` 投影到 `AbiSourceDeclarationAnchor`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AbiDeclarationAnchor {
    /// 本 publication 的 source declaration。
    Source(AbiSourceDeclarationAnchor),
    /// 跨 artifact 的外部 declaration 引用。
    External(ExternalDeclarationAnchor),
    /// std 标准库 symbol。
    Std { symbol: StdSymbolId },
}

/// `AbiDeclarationAnchor::Source` 的纯数据表示——artifact-model 侧的 source anchor 投影。
///
/// 这是 compiler 的 `SourceDeclarationAnchor` 的 ABI 镜像。字段均为稳定数据,不含 compiler
/// 内部类型。Compiler 侧 `abi.rs` 的派生函数负责投影。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AbiSourceDeclarationAnchor {
    /// owning publication 的 stable id 字符串(对应 compiler::PublicationId)。
    pub publication_id: String,
    /// ABI epoch(默认 0,普通 publication 版本变化不递增)。
    pub abi_epoch: u32,
    /// module path 分段(完整保留,不退化成序号)。
    pub module_path: Vec<String>,
    /// source symbol 名字。
    pub symbol: String,
    /// declaration kind。
    pub kind: AbiDeclarationKind,
}

/// 跨 artifact 外部 declaration anchor(架构 L669-673)。
///
/// `PublishedDeclarationId.stable_id` 生成机制尚未定义(架构 L699 只说"发布方 artifact 生成并
/// 冻结的不透明 token")。P2a 只建类型存根。见本模块顶部文档的"存根,待定"注释。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalDeclarationAnchor {
    /// 发布方 publication identity。
    pub owner_publication_id: String,
    pub owner_abi_epoch: u32,
    /// 发布方冻结的不透明 declaration id。
    pub declaration: PublishedDeclarationId,
    pub kind: AbiDeclarationKind,
}

/// 发布方 artifact 冻结的不透明 declaration token(架构 L675-677)。
///
/// **`stable_id` 生成机制未定**:牵扯发布 / registry 侧。P2a 只立类型,不实现生成逻辑。
/// 这里只用于 `ExternalDeclarationAnchor`,本 publication 的 source anchor 走
/// `AbiDeclarationAnchor::Source`,不需要 `stable_id`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublishedDeclarationId {
    /// 不透明稳定 token。consumer 不得反解析或据此重建源码名字。
    pub stable_id: String,
}

/// ABI declaration 种类(架构 L679-686)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AbiDeclarationKind {
    Type,
    Alias,
    Interface,
    Callable,
    Const,
    Instance,
}

/// 类型可命名性(架构 L688-691,L703-705)。
///
/// `ClosureOnly`:ABI-visible 但 source-unnameable。Compiler / IDE 可通过 inference 使用,
/// runtime / artifact 可用于 schema 和 link,但外部源码不能直接书写其 public name。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeNameability {
    /// 有公开 name,外部源码可书写。
    PublicNameable,
    /// ABI 可见,但源码不可书写(closure-only)。
    ClosureOnly,
}

/// Artifact wire-level ABI identity facts.
///
/// The compiler projects nominal ids into stable hex keys before crossing the
/// artifact boundary. This keeps private `AbiTypeId` key bytes opaque in Rust
/// while still making the artifact carry the required facts: public export path
/// -> ABI symbol id, and ABI type id -> nameability.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AbiIdentityFacts {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub public_symbols: BTreeMap<String, AbiSymbolIdFact>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_nameability: BTreeMap<String, TypeNameability>,
}

impl AbiIdentityFacts {
    pub fn is_empty(&self) -> bool {
        self.public_symbols.is_empty() && self.type_nameability.is_empty()
    }
}

/// Serializable ABI symbol id projection used by artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AbiSymbolIdFact {
    Type { abi_type_id: String },
    Alias { abi_alias_id: String },
    Interface { abi_interface_id: String },
    Callable { abi_callable_id: String },
    Const { abi_const_id: String },
    Instance { abi_instance_id: String },
}

/// std 标准库 symbol id(存根;实际 std symbol 枚举/命名归后续阶段)。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StdSymbolId(pub String);

/// 类型的完整 ABI fact(架构 L650-659)。
///
/// - `type_id`:nominal id,只用于 type equality。
/// - `declaration_anchor`:声明位置,nominal 的语义来源。
/// - `source_entity`:对应的 compiler source entity(Option;P2b 才消费,这里存 index/id)。
///   P2a 用 `u64` 占位(实际类型由 P2b 对接)。
/// - `public_path`:public export path(如果有)。
/// - `nameability`:是否可书写。
/// - `descriptor`:canonical type descriptor(P2a 为 opaque bytes,P3a 接入真实 descriptor)。
/// - `contract_revision`:descriptor / schema revision。
///
/// `contract_revision` 不自带 `type_id`——它的归属由本 struct 的 `type_id` 字段提供(架构 L631)。
#[derive(Debug, Clone)]
pub struct AbiTypeFact {
    pub type_id: AbiTypeId,
    pub declaration_anchor: AbiDeclarationAnchor,
    /// 对应 compiler source entity 的不透明 id(P2a 占位;P2b 替换为真实 TopLevelEntityId)。
    pub source_entity: Option<u64>,
    /// Public export path,如果有(P2a 为 Option<String> 占位)。
    pub public_path: Option<String>,
    pub nameability: TypeNameability,
    /// Canonical type descriptor(P2a 为 opaque bytes;P3a 接入真实 CanonicalTypeDescriptor)。
    pub descriptor: Vec<u8>,
    pub contract_revision: AbiContractRevision,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi_identity::derive::{abi_type_id_from_source_anchor, AbiSourceAnchorInput};

    fn make_anchor(
        pub_id: &str,
        epoch: u32,
        module: &[&str],
        symbol: &str,
    ) -> AbiSourceAnchorInput {
        AbiSourceAnchorInput {
            publication_id: pub_id.to_string(),
            abi_epoch: epoch,
            module_path: module.iter().map(|s| s.to_string()).collect(),
            symbol: symbol.to_string(),
            kind: AbiDeclarationKind::Type,
        }
    }

    /// 架构 Verification #16: same-shape ≠ equal。
    ///
    /// 两个 anchor 不同(不同 module 或 symbol)派生出的 AbiTypeId 不相等,即使字段"形状"上
    /// 相同。
    #[test]
    fn same_shape_not_equal() {
        let anchor_a = make_anchor("example.com/pkg", 0, &["foo", "bar"], "MyType");
        let anchor_b = make_anchor("example.com/pkg", 0, &["foo", "baz"], "MyType");

        let id_a = abi_type_id_from_source_anchor(&anchor_a, &[]);
        let id_b = abi_type_id_from_source_anchor(&anchor_b, &[]);

        assert_ne!(
            id_a, id_b,
            "不同 module path 的 anchor 应派生出不同 AbiTypeId"
        );
    }

    /// 架构 Verification #12(P1b→P2):move-selector 改 nominal。
    ///
    /// 同名 symbol 在不同 module_path → anchor 不同 → AbiTypeId 不同。
    #[test]
    fn move_selector_changes_nominal() {
        let anchor_before = make_anchor("example.com/pkg", 0, &["original", "module"], "User");
        let anchor_after = make_anchor("example.com/pkg", 0, &["moved", "module"], "User");

        let id_before = abi_type_id_from_source_anchor(&anchor_before, &[]);
        let id_after = abi_type_id_from_source_anchor(&anchor_after, &[]);

        assert_ne!(
            id_before, id_after,
            "symbol 移到不同 module 后 AbiTypeId 应改变"
        );
    }

    /// 架构 Verification #13: descriptor 改 contract revision 而 nominal 不变。
    ///
    /// 同一 anchor 派生的 AbiTypeId 不变;两个 descriptor 不同的 AbiContractRevision 不等,
    /// 但 nominal id 相等(分离性)。
    #[test]
    fn descriptor_change_changes_contract_revision_not_nominal() {
        let anchor = make_anchor("example.com/pkg", 0, &["api"], "Order");

        let id = abi_type_id_from_source_anchor(&anchor, &[]);
        // 同一 anchor 二次调用应返回相同 id
        let id_again = abi_type_id_from_source_anchor(&anchor, &[]);
        assert_eq!(id, id_again, "同一 anchor 派生的 AbiTypeId 应相等");

        // 两个不同 descriptor 的 contract revision 不相等
        let rev_a = AbiContractRevision {
            descriptor_hash: DescriptorHash(vec![0x01, 0x02]),
            schema_revision: SchemaRevision(1),
        };
        let rev_b = AbiContractRevision {
            descriptor_hash: DescriptorHash(vec![0x03, 0x04]),
            schema_revision: SchemaRevision(2),
        };
        assert_ne!(rev_a, rev_b, "不同 descriptor 的 contract revision 应不等");
        // nominal id 来自 anchor,与 descriptor 无关
        assert_eq!(
            id, id_again,
            "contract revision 变化不影响 nominal AbiTypeId"
        );
    }

    /// 架构 Verification: 泛型实参进 nominal。
    ///
    /// 同 anchor 但不同 type_args → 不同 AbiTypeId。
    #[test]
    fn type_args_affect_nominal_id() {
        let anchor = make_anchor("example.com/pkg", 0, &["collections"], "List");

        let inner_int = make_anchor("skiff.run/std", 0, &["std", "primitives"], "Int");
        let inner_str = make_anchor("skiff.run/std", 0, &["std", "primitives"], "String");

        let id_int = abi_type_id_from_source_anchor(&inner_int, &[]);
        let id_str = abi_type_id_from_source_anchor(&inner_str, &[]);

        let list_int = abi_type_id_from_source_anchor(&anchor, &[id_int]);
        let list_str = abi_type_id_from_source_anchor(&anchor, &[id_str]);

        assert_ne!(
            list_int, list_str,
            "不同 type args 实例化的泛型类型应有不同 AbiTypeId"
        );
    }

    /// 架构 Verification #18: alias 不建 nominal AbiTypeId。
    ///
    /// 验证类型层面:alias 走 AbiAliasId 而非 AbiTypeId,两者是不同 Rust 类型。
    #[test]
    fn alias_uses_abi_alias_id_not_type_id() {
        // 类型区分已由 Rust 类型系统保证:AbiAliasId 与 AbiTypeId 是不同 newtype
        let alias_id = AbiAliasId::from_key_bytes(vec![1, 2, 3]);
        let type_id = AbiTypeId::from_key_bytes(vec![1, 2, 3]);

        // 即使 key bytes 相同,两者是不同 enum variant
        let alias_sym = AbiSymbolId::Alias(alias_id);
        let type_sym = AbiSymbolId::Type(type_id);

        assert_ne!(
            alias_sym, type_sym,
            "Alias 和 Type 是不同 AbiSymbolId variant"
        );
        // 验证:alias 走 AbiAliasId,type 走 AbiTypeId
        assert!(matches!(alias_sym, AbiSymbolId::Alias(_)));
        assert!(matches!(type_sym, AbiSymbolId::Type(_)));
    }

    /// AbiContractRevision 不自带 AbiSymbolId:分离验证。
    #[test]
    fn contract_revision_has_no_symbol_id() {
        let rev = AbiContractRevision {
            descriptor_hash: DescriptorHash(vec![0xDE, 0xAD]),
            schema_revision: SchemaRevision(0),
        };
        // 编译时已保证:AbiContractRevision 无 type_id/symbol_id 字段
        // 这里只做运行时验证 PartialEq
        let rev2 = AbiContractRevision {
            descriptor_hash: DescriptorHash(vec![0xDE, 0xAD]),
            schema_revision: SchemaRevision(0),
        };
        assert_eq!(rev, rev2);
    }

    /// ClosureOnly 与 PublicNameable 区分。
    #[test]
    fn nameability_variants_distinct() {
        assert_ne!(
            TypeNameability::PublicNameable,
            TypeNameability::ClosureOnly
        );
        assert_eq!(TypeNameability::ClosureOnly, TypeNameability::ClosureOnly);
    }
}

/// ABI id 派生辅助(结构化 stable key 编码)。
///
/// 这个子模块提供从结构化输入到 `AbiTypeId` 等 id 的确定性派生函数。P2a 阶段用 null-byte
/// 分隔的字节序列保证"同输入 → 同 key,异输入 → 异 key"语义。跨 artifact 的 canonical
/// sha256 encoding 留给 `artifact-identity` crate(后续阶段)。
pub mod derive {
    use super::{
        AbiAliasId, AbiCallableId, AbiConstId, AbiDeclarationKind, AbiInstanceId, AbiInterfaceId,
        AbiTypeId,
    };

    /// 从 source anchor 派生 `AbiTypeId` 的输入(纯数据)。
    ///
    /// 对应 compiler 的 `SourceDeclarationAnchor`,但不包含 compiler 内部类型。Compiler 侧
    /// `abi.rs` 负责把 `SourceDeclarationAnchor` 投影成这个结构。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AbiSourceAnchorInput {
        pub publication_id: String,
        pub abi_epoch: u32,
        pub module_path: Vec<String>,
        pub symbol: String,
        pub kind: AbiDeclarationKind,
    }

    /// 从 source anchor + type args 派生 `AbiTypeId`。
    ///
    /// # 保证
    ///
    /// - 同 anchor + 同 type_args → 同 AbiTypeId。
    /// - anchor 任何字段不同 → 不同 AbiTypeId。
    /// - type_args 不同 → 不同 AbiTypeId。
    /// - **不吞入** descriptor bytes / schema hash / publication version / build id(架构 L618-620)。
    ///
    /// # 编码格式(P2a 阶段)
    ///
    /// 使用 `null-byte length-prefixed` 分段编码,确保不同字段拼接无二义性:
    /// `type:publication_id\0epoch\0module_seg1\0...\0symbol\0kind\0[type_arg_key...]\0`
    /// 后续 artifact-identity 层会替换为 sha256。
    pub fn abi_type_id_from_source_anchor(
        anchor: &AbiSourceAnchorInput,
        type_args: &[AbiTypeId],
    ) -> AbiTypeId {
        let key = encode_source_anchor_key("type", anchor, type_args);
        AbiTypeId::from_key_bytes(key)
    }

    /// 从 source anchor 派生 `AbiAliasId`。
    ///
    /// Alias 不建 `AbiTypeId`;按 target 展开(架构 L707-708)。
    pub fn abi_alias_id_from_source_anchor(anchor: &AbiSourceAnchorInput) -> AbiAliasId {
        let key = encode_source_anchor_key("alias", anchor, &[]);
        AbiAliasId::from_key_bytes(key)
    }

    /// 从 source anchor 派生 `AbiInterfaceId`。
    pub fn abi_interface_id_from_source_anchor(
        anchor: &AbiSourceAnchorInput,
        type_args: &[AbiTypeId],
    ) -> AbiInterfaceId {
        let key = encode_source_anchor_key("interface", anchor, type_args);
        AbiInterfaceId::from_key_bytes(key)
    }

    /// 从 source anchor 派生 `AbiCallableId`。
    pub fn abi_callable_id_from_source_anchor(anchor: &AbiSourceAnchorInput) -> AbiCallableId {
        let key = encode_source_anchor_key("callable", anchor, &[]);
        AbiCallableId::from_key_bytes(key)
    }

    /// 从 source anchor 派生 `AbiConstId`。
    pub fn abi_const_id_from_source_anchor(anchor: &AbiSourceAnchorInput) -> AbiConstId {
        let key = encode_source_anchor_key("const", anchor, &[]);
        AbiConstId::from_key_bytes(key)
    }

    /// 从 source anchor 派生 `AbiInstanceId`。
    pub fn abi_instance_id_from_source_anchor(anchor: &AbiSourceAnchorInput) -> AbiInstanceId {
        let key = encode_source_anchor_key("instance", anchor, &[]);
        AbiInstanceId::from_key_bytes(key)
    }

    /// 内部:结构化 null-byte 分隔 key 编码。
    ///
    /// 格式:`symbol_kind\0pub_id\0epoch_le4\0module_seg_count_le4\0[seg\0...]\0symbol\0
    ///        decl_kind\0type_arg_count_le4\0[type_arg_key_len_le4\0type_arg_key_bytes...]\0`
    ///
    /// 每个变长字段前缀 length(4 LE bytes),避免不同字段边界的二义拼接。
    fn encode_source_anchor_key(
        symbol_kind: &str,
        anchor: &AbiSourceAnchorInput,
        type_args: &[AbiTypeId],
    ) -> Vec<u8> {
        let mut buf = Vec::new();

        // symbol kind tag(type/alias/interface/callable/const/instance)
        write_framed(&mut buf, symbol_kind.as_bytes());

        // publication id
        write_framed(&mut buf, anchor.publication_id.as_bytes());

        // abi epoch(4 LE bytes,固定宽度)
        buf.extend_from_slice(&anchor.abi_epoch.to_le_bytes());
        buf.push(0); // separator

        // module path 分段
        let seg_count = anchor.module_path.len() as u32;
        buf.extend_from_slice(&seg_count.to_le_bytes());
        for seg in &anchor.module_path {
            write_framed(&mut buf, seg.as_bytes());
        }

        // symbol name
        write_framed(&mut buf, anchor.symbol.as_bytes());

        // declaration kind
        let kind_byte: u8 = match anchor.kind {
            AbiDeclarationKind::Type => 0,
            AbiDeclarationKind::Alias => 1,
            AbiDeclarationKind::Interface => 2,
            AbiDeclarationKind::Callable => 3,
            AbiDeclarationKind::Const => 4,
            AbiDeclarationKind::Instance => 5,
        };
        buf.push(kind_byte);
        buf.push(0); // separator

        // type args
        let arg_count = type_args.len() as u32;
        buf.extend_from_slice(&arg_count.to_le_bytes());
        for arg in type_args {
            write_framed(&mut buf, arg.key_bytes());
        }

        buf
    }

    /// 写入 length-prefixed 帧:4 LE bytes 长度 + 内容 + NUL 终止。
    fn write_framed(buf: &mut Vec<u8>, data: &[u8]) {
        let len = data.len() as u32;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(data);
        buf.push(0); // separator/terminator
    }
}
