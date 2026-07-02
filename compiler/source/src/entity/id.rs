//! Entity id 空间(架构 `Entity Id Spaces` 节 L147-225)。
//!
//! 核心不变量(架构 L149,L217-225):**entity id 必须有明确 owner context,不能用单个
//! 全局 `u32` 表示所有实体。** 每个 owner-local id 是严格 newtype,index 字段私有,只能
//! 经持有它的 owner-local table 构造(见 `table.rs`)。这样裸 `u32` / 裸 `3` 无法跨 file /
//! package 边界冒充 stable identity。
//!
//! 顶层 entity id 若要脱离它的 `PublicationEntityTable` 跨 artifact / package 边界流动,
//! 必须先投影为 declaration anchor / ABI nominal id / contract revision(见 `anchor.rs`
//! 与后续 P2),不能裸传 local index。

use super::kind::EntityNamespace;

/// 跨 source compile model 传递的 typed entity id(架构 L152-161)。
///
/// 每个 variant 要么自带 owner,要么指向一个 owner-local table。没有"全局裸 index"variant。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityId {
    TopLevel(TopLevelEntityId),
    ImplMethod(ImplMethodEntityId),
    Local(LocalEntityId),
    TypeParameter(TypeParameterEntityId),
    PackageCapability(PackageCapabilityEntityId),
    ExternalPackage(ExternalPackageEntityId),
    ExternalService(ExternalServiceEntityId),
    Builtin(BuiltinEntityId),
}

/// 当前 publication entity table 中的顶层声明 handle(架构 L172-175)。
///
/// `publication_local_index` 只是当前 compiler table 的 owner-local handle——不是 ABI
/// nominal identity 输入,也不是 source declaration anchor(架构 L177-181)。它只能在所属
/// `PublicationEntityTable` owner 上下文内解释。
///
/// 字段私有:只能由 `PublicationEntityTable::push` 之类的 owner 方法构造(见 `table.rs`),
/// 防止从任意 `u32` 凭空造一个"顶层 entity"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TopLevelEntityId {
    publication_local_index: u32,
}

impl TopLevelEntityId {
    /// 仅供同 crate 的 owner table 构造。不对外暴露任意 index 入口。
    pub(super) fn from_owner_index(publication_local_index: u32) -> Self {
        Self {
            publication_local_index,
        }
    }

    /// owner-local handle 的数值投影。**调用方必须自带 owner context**(即知道它属于哪个
    /// `PublicationEntityTable`);裸取这个数字跨界使用违反架构 L217-225。
    pub(super) fn owner_local_index(self) -> u32 {
        self.publication_local_index
    }
}

/// receiver method namespace 中的 impl method entity(架构 L99,L309)。
///
/// `impl` method 不进入顶层 source selector,因此它不独立持有 source declaration anchor;
/// 它的身份由 owning type / interface 的 anchor + method name 在该 owner 的 descriptor 内
/// 承载。这里仍给它一个 owner-local id 供 name resolution / 诊断使用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImplMethodEntityId {
    owner: TopLevelEntityId,
    method_index: u32,
}

impl ImplMethodEntityId {
    pub(super) fn from_owner_index(owner: TopLevelEntityId, method_index: u32) -> Self {
        Self {
            owner,
            method_index,
        }
    }

    pub(super) fn owner(self) -> TopLevelEntityId {
        self.owner
    }

    pub(super) fn method_index(self) -> u32 {
        self.method_index
    }
}

/// 一个 executable / const initializer / callback / pattern owner 的标识(架构 L183-196)。
///
/// `LocalEntityId` 永远绑定到某个 `LocalEntityOwnerId`,因此 `local_index` 不会被误当作
/// 全局序号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalEntityOwnerId {
    index: u32,
}

impl LocalEntityOwnerId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// 局部 entity(参数 / 局部变量 / pattern binding / catch binding 等,架构 L192-196)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalEntityId {
    owner: LocalEntityOwnerId,
    local_index: u32,
}

impl LocalEntityId {
    pub(super) fn from_owner_index(owner: LocalEntityOwnerId, local_index: u32) -> Self {
        Self { owner, local_index }
    }

    pub(super) fn owner(self) -> LocalEntityOwnerId {
        self.owner
    }

    pub(super) fn local_index(self) -> u32 {
        self.local_index
    }
}

/// type / function / interface / method 的 generic owner(架构 L198-205 的 `GenericOwnerId`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenericOwnerId {
    index: u32,
}

impl GenericOwnerId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// type parameter entity(架构 L200-205)。属于某个 generic owner 的 type namespace。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeParameterEntityId {
    owner: GenericOwnerId,
    param_index: u32,
}

impl TypeParameterEntityId {
    pub(super) fn from_owner_index(owner: GenericOwnerId, param_index: u32) -> Self {
        Self { owner, param_index }
    }

    pub(super) fn owner(self) -> GenericOwnerId {
        self.owner
    }

    pub(super) fn param_index(self) -> u32 {
        self.param_index
    }
}

/// package capability alias 引入的受控 receiver root entity(架构 L102,EntityKind
/// `PackageCapability`)。可在 package source 中被调用,但不是 first-class runtime value。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageCapabilityEntityId {
    index: u32,
}

impl PackageCapabilityEntityId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// dependency package public symbol 在当前 compilation 中的引用实体(架构 L103,
/// `Package And Service References` 节 L411-418)。lower 到 local linkage。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalPackageEntityId {
    index: u32,
}

impl ExternalPackageEntityId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// service dependency operation / public instance 引用实体(架构 L104,L420-427)。
///
/// 是 remote linkage entity,**不能与 package local symbol 合并**(架构 L429-434):即使
/// display string 相同,package callable 与 service operation 也必须产生不同 entity kind。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExternalServiceEntityId {
    index: u32,
}

impl ExternalServiceEntityId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// compiler-known platform symbol registry 的标识(架构 L209-215)。
///
/// P0 只立类型骨架——`BuiltinEntityId` 由 `(registry, symbol, namespace)` 三元组组成。把
/// 现有 `prelude_registry` 的硬编码 root(`"std"|"ext"|...`)接进来,以及定义具体有哪些
/// builtin symbol,归 P1a,不在 P0 触碰。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuiltinRegistryId {
    index: u32,
}

impl BuiltinRegistryId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// 某个 builtin registry 内的 symbol 序号(P0 骨架)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuiltinSymbolId {
    index: u32,
}

impl BuiltinSymbolId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// std / prelude / compiler-known built-in symbol 的 entity id(架构 L209-215)。
///
/// 携带 namespace:同一 builtin spelling 在 value / type namespace 是不同 entity。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuiltinEntityId {
    registry: BuiltinRegistryId,
    symbol: BuiltinSymbolId,
    namespace: EntityNamespace,
}

impl BuiltinEntityId {
    pub(super) fn new(
        registry: BuiltinRegistryId,
        symbol: BuiltinSymbolId,
        namespace: EntityNamespace,
    ) -> Self {
        Self {
            registry,
            symbol,
            namespace,
        }
    }

    pub(super) fn registry(self) -> BuiltinRegistryId {
        self.registry
    }

    pub(super) fn symbol(self) -> BuiltinSymbolId {
        self.symbol
    }

    pub fn namespace(self) -> EntityNamespace {
        self.namespace
    }
}
