//! Compiler entity & identity model (P0 skeleton + P1a typed resolution).
//!
//! 本模块逐字落地 `doc/architecture/compiler-entity-and-identity.md` 的 Entity Id
//! Spaces / Entity Model / Resolver Roots / NamePath Resolution / Declaration Anchor
//! 几节的 Rust-ish 草图。架构文档明确这些类型不是 public API,只是 compiler 内部分层。
//!
//! P0 阶段约束:
//!
//! - **只定义类型,不改任何消费方。** 现有 `source_name_resolution.rs` /
//!   `source_identity.rs` / `semantic/` 不在本阶段触碰。
//! - id 是严格 newtype,字段私有,只能经持有它的 owner-local table 构造,杜绝裸
//!   `u32` 跨 file / package 边界流动(架构 Entity Id Spaces 节:"裸 `3` 不是 stable
//!   identity")。
//! - builtin 这里只立类型骨架;接现有 `prelude_registry` 的硬编码 root 列表归 P1a。
//!
//! P1a 阶段(typed resolution)新增 `resolve` 子模块,落地 `ResolutionEnv` /
//! `resolve_dotted_path` / `is_unresolved_root`,并把 `source_name_resolution.rs` 的
//! 诊断 pass 改为经由新 resolver 驱动。见 `resolve.rs` 模块注释。
//!
//! `#[allow(dead_code, unused_imports)]` 覆盖整个模块:P0/P1a 期间不是所有导出项都有
//! 消费方,随 P1b/P2/P3b 起步后自然收缩。
#![allow(dead_code, unused_imports)]

pub mod abi;
mod anchor;
mod id;
mod kind;
mod name;
mod resolution;
mod table;

pub mod resolve;

#[cfg(test)]
mod tests;

pub use anchor::{
    AbiEpoch, ModulePath as AnchorModulePath, PublicationIdentity, SourceDeclarationAnchor,
    SourceDeclarationKind, SourceSelector, SymbolName,
};
pub use id::{
    BuiltinEntityId, BuiltinRegistryId, BuiltinSymbolId, EntityId, ExternalPackageEntityId,
    ExternalServiceEntityId, GenericOwnerId, ImplMethodEntityId, LocalEntityId, LocalEntityOwnerId,
    PackageCapabilityEntityId, TopLevelEntityId, TypeParameterEntityId,
};
pub use kind::{EntityKind, EntityNamespace};
pub use name::{EntityName, EntityRef, NameOwner, NamePath};
pub use resolution::{PathPrefixId, ResolvedCallable, ResolvedPath, ResolverRoot, ResolverRootId};
pub use resolve::{
    builtin_entity_for_symbol, intrinsic_resolver_root_id, is_unresolved_root, resolve_dotted_path,
    stable_name_index, ResolutionEnv, MODULE_PREFIX_MARKER, TOP_LEVEL_REF_MARKER,
};
pub use table::{
    LexicalScope, LexicalScopeId, LocalEntity, LocalEntityOwner, LocalEntityTable, ModulePath,
    ModulePathIndex, PublicationEntityModel, PublicationEntityTable, PublicationKind,
    ResolverRootTable, TopLevelEntity,
};
