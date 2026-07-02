//! Entity 名字入口与引用(架构 `Entity Names` 节 L108-145)。
//!
//! 一个 entity 可以有多个名字入口(`EntityName`),但名字入口不是 entity 本身——同一函数
//! 既能被 `root.internal.user.getUser` 找到,又能被 consumer 经 `pkg.user.get` 找到,这两条
//! 是不同 lookup context 下的入口,不复制出两个 source entity。
//!
//! 解析完成后,引用点持有 `EntityRef`(typed),不再保存 display path 作为语义 key
//! (架构 L137-138)。

use crate::shared::error::SourceSpan;

use super::id::EntityId;
use super::kind::EntityNamespace;

/// `a.b.c` 这类分段路径(架构 Core Invariant 的 `NamePath`)。
///
/// P0 用结构化的 segment 向量承载;具体 spelling 校验 / interning 留待后续阶段。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NamePath {
    segments: Vec<String>,
}

impl NamePath {
    pub(super) fn new(segments: Vec<String>) -> Self {
        Self { segments }
    }

    pub(super) fn segments(&self) -> &[String] {
        &self.segments
    }
}

/// 名字入口的拥有者上下文(producer source scope vs consumer dependency view 等)。
///
/// P0 骨架:variant 形状随后续 lookup 实现细化,这里只立"名字入口有 owner"这件事,呼应
/// 架构 L116-122 同一 entity 在不同 lookup context 下有不同名字入口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameOwner {
    /// 当前 publication 自己的 source lookup scope。
    CurrentPublication,
    /// 经某个 dependency 视角看到的名字入口(后续阶段携带 dependency 标识)。
    Dependency,
}

/// 某个 scope / path 中指向 entity 的名字入口(架构 L123-129)。仅用于 lookup。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityName {
    owner: NameOwner,
    path: NamePath,
    target: EntityId,
    namespace: EntityNamespace,
}

impl EntityName {
    pub(super) fn new(
        owner: NameOwner,
        path: NamePath,
        target: EntityId,
        namespace: EntityNamespace,
    ) -> Self {
        Self {
            owner,
            path,
            target,
            namespace,
        }
    }

    pub(super) fn target(&self) -> EntityId {
        self.target
    }

    pub(super) fn namespace(&self) -> EntityNamespace {
        self.namespace
    }
}

/// 某个 use site 解析后的 entity 引用(架构 L140-145)。
///
/// 解析完成后,AST / HIR / source facts 持有 `EntityRef` 而不是 display path。`use_site`
/// 复用现有 `error::SourceSpan`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityRef {
    target: EntityId,
    use_site: SourceSpan,
}

impl EntityRef {
    pub(super) fn new(target: EntityId, use_site: SourceSpan) -> Self {
        Self { target, use_site }
    }

    pub(super) fn target(self) -> EntityId {
        self.target
    }

    pub(super) fn use_site(self) -> SourceSpan {
        self.use_site
    }
}
