//! Resolver roots 与 typed name resolution 结果(架构 `Resolver Roots` 节 L342-369、
//! `NamePath Resolution` 节 L371-404、`Package And Service References` 节 L406-451)。
//!
//! 核心不变量(架构 Core Invariant L49-62):`root` / `std` / package alias / service alias
//! 是 resolver root,**不是 entity**;`root.user` / `pkg.http` 这类中间前缀也不是 entity。
//! 只有完整路径的最终结果才能成为 entity ref。因此 resolution 必须产出 typed
//! `ResolvedPath`,consumer 不能把 `PathPrefix` / `ResolverRoot` 当 entity fallback。

use super::id::{
    EntityId, ExternalPackageEntityId, ExternalServiceEntityId, ImplMethodEntityId,
    TopLevelEntityId,
};

/// name / path lookup 的起点(架构 L346-353)。**不是 entity**,不能作为表达式或类型引用的
/// 最终结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolverRoot {
    /// `root`:当前 publication source set 的 module / top-level lookup。
    CurrentPublicationRoot,
    /// `std`:compiler-provided platform namespace。
    StdRoot,
    /// package alias:dependency package public surface,后续调用走 local linkage。
    PackageDependency {
        alias: PackageAlias,
        dependency_slot: u32,
    },
    /// service alias:service dependency public operation / instance surface,后续走 remote linkage。
    ServiceDependency {
        alias: ServiceAlias,
        dependency_slot: u32,
    },
    /// `config`:publication config requirement API,不是普通 runtime object。
    ConfigRoot,
}

/// package resolver root 的别名 spelling(P0 骨架,后续阶段做校验 / interning)。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageAlias(String);

impl PackageAlias {
    pub(super) fn new(alias: String) -> Self {
        Self(alias)
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// service resolver root 的别名 spelling(P0 骨架)。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceAlias(String);

impl ServiceAlias {
    pub(super) fn new(alias: String) -> Self {
        Self(alias)
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// 已注册的 resolver root 在 root table 中的 handle(owner = `ResolverRootTable`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResolverRootId {
    index: u32,
}

impl ResolverRootId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// qualified lookup 的中间前缀(如 `pkg.user`、`root.internal`)在 path-prefix table 中的
/// handle。中间前缀**不是 entity**(架构 L376-391),不要求唯一对应某个实体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathPrefixId {
    index: u32,
}

impl PathPrefixId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}

/// 一次 name path 解析的 typed 结果(架构 L396-404)。
///
/// consumer 只能接受它所在 context 允许的最终结果;**不能把 `PathPrefix` 或 `ResolverRoot`
/// 当 entity fallback**。这是 P1a verification "root 不能当 entity final result" 与
/// "path prefix 不当 entity" 两条 case 的类型地基。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPath {
    Entity(EntityId),
    PathPrefix(PathPrefixId),
    ResolverRoot(ResolverRootId),
}

impl ResolvedPath {
    /// 取最终 entity;若解析停在 prefix / root 则为 `None`。
    ///
    /// 这是架构 L403-404 "不能把 PathPrefix / ResolverRoot 当 entity fallback" 的程序化入口:
    /// 需要 entity 的 consumer 走这里,拿不到就报诊断,而不是凑一个 fallback。
    pub(super) fn as_entity(&self) -> Option<EntityId> {
        match self {
            ResolvedPath::Entity(id) => Some(*id),
            ResolvedPath::PathPrefix(_) | ResolvedPath::ResolverRoot(_) => None,
        }
    }
}

/// lowering 在 call position 消费 name resolution 得到的 call target(架构 L438-446)。
///
/// runtime 不再通过字符串判断一次 call 是 package call 还是 service call——kind 已在这里定型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedCallable {
    LocalFunction(TopLevelEntityId),
    LocalImplMethod(ImplMethodEntityId),
    PackageFunction(ExternalPackageEntityId),
    ServiceOperation(ExternalServiceEntityId),
    Builtin(BuiltinCallableId),
}

/// `BuiltinEntityId` 在 call position 的投影(架构 L450-452)。
///
/// 它是 call-target 视角的 builtin 句柄,不是另一个独立 id space;二者必须能互相对应。
/// builtin 不进入 `AbiSymbolId`。P0 只立类型骨架。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuiltinCallableId {
    index: u32,
}

impl BuiltinCallableId {
    pub(super) fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub(super) fn index(self) -> u32 {
        self.index
    }
}
