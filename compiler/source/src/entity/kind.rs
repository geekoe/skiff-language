//! Entity kind 与 namespace。架构 `Entity Model` 节(L75-94)与 `Entity Names` 节
//! (L131-135)。
//!
//! `EntityKind` 表示 compiler 可解析到的实体类别;它不是 runtime tag。`EntityNamespace`
//! 区分 value / type 两个名字空间——架构 `Type And Value Namespaces` 节要求 lookup key
//! 必须携带 namespace,不能只按短名匹配。

/// compiler 可解析到的实体类别(架构 L76-94)。
///
/// 分类规则见架构 `Entity Model` 节 L96-106:
/// - 顶层 `type`/`alias`/`interface`/`function`/`const`/DB object 是 publication source entity;
/// - `impl` method 在 receiver method namespace,不进顶层 source selector;
/// - 参数 / 局部 / pattern binding 是 local entity;
/// - type parameter 是 type namespace 中的 local entity;
/// - package capability alias 是受控 receiver root entity;
/// - external package / service entity 是 dependency 引用,不能与 local symbol 合并;
/// - std/prelude/compiler-known built-in 解析为 `Builtin`,不是 `root` source entity。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityKind {
    Type,
    Alias,
    Interface,
    Function,
    ImplMethod,
    Const,
    DbObject,
    Local,
    Parameter,
    PatternBinding,
    TypeParameter,
    PackageCapability,
    ExternalPackageSymbol,
    ExternalServiceOperation,
    ExternalServiceInstance,
    BuiltinSymbol,
}

/// value / type 名字空间(架构 L131-135)。
///
/// 同一 source spelling 可以在不同 namespace 解析为不同 entity(架构
/// `Type And Value Namespaces` 节 L477-478),前提是用户语义允许。解析结果必须携带
/// namespace,后续阶段不得只按短名匹配。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityNamespace {
    Value,
    Type,
}
