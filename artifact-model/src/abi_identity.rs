//! ABI nominal identity 类型层(架构 `ABI Nominal Identity And Contract Revision` L595-637、
//! `ABI Type Identity` L639-712)。
//!
//! 本模块只定义 ABI identity 的 typed inputs/newtypes 与 artifact DTO。所有 key encoding、
//! canonical bytes、hash、hex 和 stable string 派生都由 `artifact-identity` crate 负责。
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AbiTypeId {
    /// Opaque canonical key bytes assigned by `artifact-identity`.
    key: Vec<u8>,
}

impl AbiTypeId {
    /// Constructs the typed id from bytes produced by `artifact-identity`.
    pub fn from_key_bytes(key: Vec<u8>) -> Self {
        Self { key }
    }

    /// Exposes the opaque bytes to the canonical identity owner and typed consumers.
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
