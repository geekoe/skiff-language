//! P1a typed name resolution。把"点号路径里未解析的 root 校验"这件原本只产诊断字符串的
//! 事,升级成产 typed `ResolvedPath`(架构 `NamePath Resolution` 节 L371-404、
//! `Package And Service References` 节 L406-451)。
//!
//! 架构核心不变量(Core Invariant L49-62):`root`/`std`/package alias/service alias 是
//! resolver root,不是 entity;中间前缀(`pkg.user`、`root.internal`)也不是 entity。只有
//! 完整路径的最终结果才能成为 entity ref。因此本 resolver 的返回类型是三态的
//! `ResolvedPath { Entity | PathPrefix | ResolverRoot }`,consumer 不能把 prefix/root 当
//! entity fallback。
//!
//! 范围约束(P1a 决策):**只产出 typed 结果 + 测试验证,不重连 lowering。**
//! lowering 消费 entity ref 归 P3b。
//!
//! **package alias ≠ service alias**(架构 case #8,L429-434):现有诊断 pass 把两类依赖
//! 别名和 module root 摊平成一个 `BTreeSet`,无法区分,本就违反"同 display path 不同
//! entity kind"。P1a 的 env 把 package aliases / service aliases 拆成独立集合,resolver
//! 据此产 `ExternalPackage` vs `ExternalService` 不同 entity——这是 case #6/#7/#8 的地基。
//!
//! builtin 处理(P1a):root 是否 builtin 查 `prelude_registry()` + 一份显式 intrinsic
//! root 表,取代旧 `matches!("std"|"ext"|...)`。builtin 编号此阶段是稳定占位(builtin
//! 不进 AbiSymbolId,架构 L450-452)。

use std::collections::BTreeSet;

use super::id::{
    BuiltinEntityId, BuiltinRegistryId, BuiltinSymbolId, EntityId, ExternalPackageEntityId,
    ExternalServiceEntityId, GenericOwnerId, LocalEntityId, LocalEntityOwnerId,
    TypeParameterEntityId,
};
use super::kind::EntityNamespace;
use super::resolution::{PathPrefixId, ResolvedPath, ResolverRootId};
use super::table::PublicationEntityTable;

/// resolver 的输入环境。
///
/// 与旧 pass 的关键差异:不再用单个摊平的 `module_roots`,而是把 lookup 起点按架构分类:
/// 当前 publication module roots、package 依赖别名、service 依赖别名各成一集。这样
/// resolver 才能对 `pkg.x` 与 `svc.x` 产出不同 entity kind(case #8)。
///
/// `local_owner` 给定时,value namespace 内 scope 可见名字解析为 `Local` entity(由该
/// owner 铸 id);为 `None` 时表示顶层位置(无局部作用域)。
///
/// `type_param_owner` 给定时,type namespace 内名字可解析为 `TypeParameter` entity(P1a
/// 只验 kind,真实 param_index 由持有 table 的上层填)。
pub struct ResolutionEnv<'a> {
    /// 当前 publication 内的 module roots(如 `user`、`billing` 等模块名第一段)。
    module_roots: &'a BTreeSet<String>,
    /// package 依赖别名集(来自 manifest dependencies)。解析为 ExternalPackage entity。
    package_aliases: &'a BTreeSet<String>,
    /// service 依赖别名集(来自 service.yml dependencies)。解析为 ExternalService
    /// entity——与 package alias 同 display path 时产出不同 entity kind(case #8)。
    service_aliases: &'a BTreeSet<String>,
    /// value namespace 中 scope 可见的名字(参数、局部变量、顶层函数/const 等)。
    value_names: &'a BTreeSet<String>,
    /// type namespace 中 scope 可见的名字(顶层 type/alias/interface、type param 等)。
    type_names: &'a BTreeSet<String>,
    /// 当前局部作用域 owner。`Some` 时 value_names 命中产 `Local` entity;`None` 顶层。
    local_owner: Option<LocalEntityOwnerId>,
    /// 当前 generic owner。`Some` 时 type_names 命中可产 `TypeParameter` entity。
    type_param_owner: Option<GenericOwnerId>,
    /// 当前 publication 的顶层声明表。存在时,`root.<module>.<symbol>` 可落到
    /// `EntityId::TopLevel`;不存在时 resolver 只做 root/prefix 分类,供诊断 pass 复用。
    current_publication_entities: Option<&'a PublicationEntityTable>,
}

impl<'a> ResolutionEnv<'a> {
    /// 构造 ResolutionEnv。
    pub fn new(
        module_roots: &'a BTreeSet<String>,
        package_aliases: &'a BTreeSet<String>,
        service_aliases: &'a BTreeSet<String>,
        value_names: &'a BTreeSet<String>,
        type_names: &'a BTreeSet<String>,
        local_owner: Option<LocalEntityOwnerId>,
        type_param_owner: Option<GenericOwnerId>,
    ) -> Self {
        Self {
            module_roots,
            package_aliases,
            service_aliases,
            value_names,
            type_names,
            local_owner,
            type_param_owner,
            current_publication_entities: None,
        }
    }

    pub fn with_current_publication_entities(
        mut self,
        top_level_entities: &'a PublicationEntityTable,
    ) -> Self {
        self.current_publication_entities = Some(top_level_entities);
        self
    }

    fn names_for(&self, namespace: EntityNamespace) -> &BTreeSet<String> {
        match namespace {
            EntityNamespace::Value => self.value_names,
            EntityNamespace::Type => self.type_names,
        }
    }
}

/// 一段点号路径(`a.b.c`)在给定 namespace 下的解析结果(架构 L376-404)。
///
/// 解析口径,**按 root 分类先后**:
/// 1. intrinsic resolver root(`root`/`std`/`config`/...):本身不是 entity。`std.<symbol>`
///    若 prelude_registry 命中则落 builtin entity;否则单段 → `ResolverRoot`、含点号 →
///    `PathPrefix`。
/// 2. package 依赖别名:`pkg.<...>` → `ExternalPackage` entity(local linkage)。
/// 3. service 依赖别名:`svc.<...>` → `ExternalService` entity(remote linkage)。**与 2 不
///    同 kind**,即使 display path 相同(case #8)。
/// 4. prelude native symbol root → builtin entity。
/// 5. 当前 scope 可见名字:
///    - value namespace + `local_owner` → `Local` entity(铸 owner-bound id)。
///    - type namespace + `type_param_owner` → `TypeParameter` entity(占位 id;真实
///      param_index 由持有 table 的上层填——P1a 范围内只验 kind)。
///    - 顶层位置(无 owner)→ `PathPrefix(TOP_LEVEL_REF_MARKER)`,表示"顶层已解析引用,
///      待上层 table 填 TopLevelEntityId"。**注意**:这里用 `TOP_LEVEL_REF_MARKER` 与普
///      通中间前缀 `MODULE_PREFIX_MARKER` 区分,上层可凭此识别语义,绝不退化成 case #3。
/// 6. module root(当前 publication 内模块名第一段):中间前缀,不是 entity;最终 entity 由
///    上层 table 填。
/// 7. 否则 → 未解析 root,返回 `None`,诊断 pass 报违例。
///
/// 关键不变量(架构 L403-404):
/// - `PathPrefix` **只**用于真正的中间前缀或顶层已解析占位。case #4(local 是 entity)走
///   `Entity(Local)`;case #3(`pkg.user` prefix 不是 entity)走 `PathPrefix`。这避免把
///   二者混为同一返回值。
pub fn resolve_dotted_path(
    env: &ResolutionEnv<'_>,
    path: &str,
    namespace: EntityNamespace,
) -> Option<ResolvedPath> {
    let root = path.split('.').next().filter(|s| !s.is_empty())?;

    // 1) intrinsic resolver root。
    if let Some(root_id) = intrinsic_resolver_root_id(root) {
        // `std.<symbol>` → builtin entity(若 prelude_registry 命中具体 symbol)。
        if root == STD_ROOT {
            if let Some(entity) = resolve_std_symbol(path, namespace) {
                return Some(ResolvedPath::Entity(entity));
            }
        }
        // `root.<module>.<symbol>` → current-publication top-level entity when the owner table is
        // available. `root` itself remains a ResolverRoot and `root.<module>` remains PathPrefix.
        if root == CURRENT_PUBLICATION_ROOT {
            return resolve_current_publication_root_path(env, path, namespace, root_id);
        }
        // 单段(如裸 `root`、`config`) → ResolverRoot;含点号 → PathPrefix。
        if path.contains('.') {
            return Some(ResolvedPath::PathPrefix(PathPrefixId::from_index(root_id)));
        }
        return Some(ResolvedPath::ResolverRoot(ResolverRootId::from_index(
            root_id,
        )));
    }

    // 2) package 依赖别名 → ExternalPackage(local linkage)。
    if env.package_aliases.contains(root) {
        return Some(ResolvedPath::Entity(EntityId::ExternalPackage(
            ExternalPackageEntityId::from_index(stable_name_index(path)),
        )));
    }

    // 3) service 依赖别名 → ExternalService(remote linkage)。与 2 不同 kind(case #8)。
    if env.service_aliases.contains(root) {
        return Some(ResolvedPath::Entity(EntityId::ExternalService(
            ExternalServiceEntityId::from_index(stable_name_index(path)),
        )));
    }

    // 4) prelude native symbol root → builtin entity。
    if crate::shared::prelude_registry::prelude_registry().is_native_symbol_root(root) {
        return Some(ResolvedPath::Entity(builtin_entity_for_root(
            root, namespace,
        )));
    }

    // 5) 当前 scope 可见名字。
    if env.names_for(namespace).contains(root) {
        return Some(resolve_visible_name(env, root, namespace));
    }

    // 6) module root:中间前缀,不是 entity;最终 entity 由上层 table 填。
    if env.module_roots.contains(root) {
        return Some(ResolvedPath::PathPrefix(PathPrefixId::from_index(
            MODULE_PREFIX_MARKER,
        )));
    }

    // 7) 未解析 root。
    None
}

/// 判定一段点号路径在给定 namespace 下是否"未解析 root 违例"(诊断 pass 的口径)。
///
/// 这是 P1a 对旧 `is_builtin_dotted_expr_root` + scope 命中判断的 typed 重写:解析成功
/// (任何 `ResolvedPath`)即非违例;`None` 即违例。
pub fn is_unresolved_root(env: &ResolutionEnv<'_>, path: &str, namespace: EntityNamespace) -> bool {
    resolve_dotted_path(env, path, namespace).is_none()
}

/// scope 内可见名字的解析:
/// - value namespace + `local_owner` → `Local` entity(铸 owner-bound id)。
/// - type namespace + `type_param_owner` → `TypeParameter` entity(占位 id)。
/// - 否则(顶层位置)→ `PathPrefix(TOP_LEVEL_REF_MARKER)`,表示"顶层已解析引用"。
fn resolve_visible_name(
    env: &ResolutionEnv<'_>,
    root: &str,
    namespace: EntityNamespace,
) -> ResolvedPath {
    match namespace {
        EntityNamespace::Value => {
            if let Some(owner) = env.local_owner {
                return ResolvedPath::Entity(EntityId::Local(LocalEntityId::from_owner_index(
                    owner,
                    stable_name_index(root),
                )));
            }
        }
        EntityNamespace::Type => {
            if let Some(owner) = env.type_param_owner {
                // P1a 只验 kind:真实 param_index 由持有 table 的上层填;此处给占位序号。
                return ResolvedPath::Entity(EntityId::TypeParameter(
                    TypeParameterEntityId::from_owner_index(owner, stable_name_index(root)),
                ));
            }
        }
    }
    // 顶层位置或无 owner:TOP_LEVEL_REF_MARKER 区分于普通 module 前缀。
    ResolvedPath::PathPrefix(PathPrefixId::from_index(TOP_LEVEL_REF_MARKER))
}

const STD_ROOT: &str = "std";
const CURRENT_PUBLICATION_ROOT: &str = "root";

/// 顶层已解析引用占位序号。
///
/// 表示"已解析的顶层 source entity 引用,待上层 table 填 TopLevelEntityId"。
/// **不是**普通中间前缀:上层可凭此值与 `MODULE_PREFIX_MARKER` 区分语义。
pub const TOP_LEVEL_REF_MARKER: u32 = u32::MAX - 1;

/// 普通 module-path 中间前缀占位序号(如 `user.getUser` 里的 `user`)。
pub const MODULE_PREFIX_MARKER: u32 = u32::MAX;

fn resolve_current_publication_root_path(
    env: &ResolutionEnv<'_>,
    path: &str,
    namespace: EntityNamespace,
    root_id: u32,
) -> Option<ResolvedPath> {
    let mut segments = path.split('.');
    let root = segments.next()?;
    debug_assert_eq!(root, CURRENT_PUBLICATION_ROOT);
    let source_segments = segments.collect::<Vec<_>>();

    if source_segments.is_empty() {
        return Some(ResolvedPath::ResolverRoot(ResolverRootId::from_index(
            root_id,
        )));
    }
    if source_segments.len() == 1 {
        return Some(ResolvedPath::PathPrefix(PathPrefixId::from_index(root_id)));
    }

    let symbol = source_segments.last().copied()?;
    let module_path = source_segments[..source_segments.len() - 1].join(".");
    if let Some(top_level_entities) = env.current_publication_entities {
        if let Some(entity) = top_level_entities
            .resolve_source_selector(&module_path, symbol, namespace)
            .map(|id| ResolvedPath::Entity(EntityId::TopLevel(id)))
        {
            return Some(entity);
        }

        for prefix_len in (2..=source_segments.len()).rev() {
            let prefix_path = source_segments[..prefix_len].join(".");
            if top_level_entities.contains_source_module_path(&prefix_path) {
                return Some(ResolvedPath::PathPrefix(PathPrefixId::from_index(root_id)));
            }
        }

        return None;
    }

    Some(ResolvedPath::PathPrefix(PathPrefixId::from_index(root_id)))
}

/// intrinsic resolver root 表(取代旧 `is_intrinsic_dotted_expr_root` 的散落 `matches!`)。
///
/// 序号固定 = 稳定身份,不可随意重排。builtin 不进 AbiSymbolId(架构 L450-452)。
pub fn intrinsic_resolver_root_id(root: &str) -> Option<u32> {
    const INTRINSIC_ROOTS: &[&str] = &[
        "std",     // 0
        "ext",     // 1
        "connect", // 2
        "config",  // 3
        "values",  // 4
        "root",    // 5
        "service", // 6
        "object",  // 7
        "db",      // 8
    ];
    INTRINSIC_ROOTS
        .iter()
        .position(|candidate| *candidate == root)
        .map(|index| index as u32)
}

/// `std.<...>` 落到 compiler-known symbol 时构造 builtin entity;未命中 → `None`。
fn resolve_std_symbol(path: &str, namespace: EntityNamespace) -> Option<EntityId> {
    let registry = crate::shared::prelude_registry::prelude_registry();
    let symbol = path.rsplit('.').next()?;
    let is_known = match namespace {
        EntityNamespace::Type => {
            registry.is_native_type_name(symbol) || registry.is_prelude_type_name(symbol)
        }
        EntityNamespace::Value => registry.is_native_symbol(symbol),
    };
    if !is_known {
        return None;
    }
    Some(builtin_entity_for_symbol(STD_ROOT, symbol, namespace))
}

fn builtin_entity_for_root(root: &str, namespace: EntityNamespace) -> EntityId {
    builtin_entity_for_symbol(root, root, namespace)
}

/// 由 (registry-root, symbol, namespace) 构造 `BuiltinEntityId`。
///
/// P1a 编号只需"同则同、异则异",不进 ABI(架构 L450-452)。
/// - `registry_index`:优先用 `intrinsic_resolver_root_id` 取固定序号;非 intrinsic root
///   用 `NATIVE_ROOT_BASE + hash`。
pub fn builtin_entity_for_symbol(root: &str, symbol: &str, namespace: EntityNamespace) -> EntityId {
    let registry_index = intrinsic_resolver_root_id(root).unwrap_or_else(|| {
        const NATIVE_ROOT_BASE: u32 = 1_000;
        NATIVE_ROOT_BASE.wrapping_add(stable_name_index(root))
    });
    EntityId::Builtin(BuiltinEntityId::new(
        BuiltinRegistryId::from_index(registry_index),
        BuiltinSymbolId::from_index(stable_name_index(symbol)),
        namespace,
    ))
}

/// 名字 → 稳定序号占位(P1a 用,不进 ABI)。FNV-1a 32-bit:"同名同号、异名异号"。
pub fn stable_name_index(name: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}
