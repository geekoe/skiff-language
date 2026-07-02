//! P0 骨架 test + P1a typed resolution 9 条 case。
//!
//! P0 的验收口径是"签名与类型必须立住"(实现文档 P0 任务卡)。这两条 case 的断言用本阶段
//! 已落地的类型即可成立,因此不留 `todo!()`;它们锁住的是后续 P1a 要保持的形状,不是
//! resolution 算法本身。
//!
//! P1a 新增 9 条 case(Verification 对照表 P1a 行):验证 typed resolver 正确分类各种 root
//! 和路径,不允许 resolver root / path prefix 冒充 entity final result。

use std::collections::BTreeSet;

use super::id::{
    BuiltinEntityId, BuiltinRegistryId, BuiltinSymbolId, EntityId, GenericOwnerId,
    LocalEntityOwnerId,
};
use super::kind::{EntityKind, EntityNamespace};
use super::resolution::{ResolvedPath, ResolverRoot, ResolverRootId};
use super::resolve::{
    resolve_dotted_path, ResolutionEnv, MODULE_PREFIX_MARKER, TOP_LEVEL_REF_MARKER,
};
use super::table::{LexicalScope, PublicationEntityTable, ResolverRootTable, TopLevelEntity};
use super::{
    AnchorModulePath, PublicationIdentity, SourceDeclarationAnchor, SourceDeclarationKind,
    SourceSelector, SymbolName,
};
use crate::shared::id::PublicationId;

// ── P0 骨架 tests ────────────────────────────────────────────────────────────

/// 架构 L860:`root` 解析为 `ResolverRoot::CurrentPublicationRoot`,且不能当 entity final
/// result。
///
/// 这里锁住的不变量:`ResolvedPath::ResolverRoot(..)` 经 `as_entity()` 得到 `None`——需要
/// entity 的 consumer 无法把 resolver root 当 fallback(架构 L403-404)。
#[test]
fn root_resolver_root_is_not_an_entity_final_result() {
    let mut roots = ResolverRootTable::new();
    let root_index = roots.push(ResolverRoot::CurrentPublicationRoot);
    let resolved = ResolvedPath::ResolverRoot(ResolverRootId::from_index(root_index));

    assert!(
        resolved.as_entity().is_none(),
        "resolver root 不能作为 entity final result"
    );
}

/// 架构 L477-478 / L327-330:同一 source spelling 在 value 与 type namespace 可解析为不同
/// entity,lexical scope 的 lookup key 必须携带 namespace,不能只按短名匹配。
///
/// 这里用同一个 spelling `Foo` 在两个 namespace 各绑一个 builtin entity,验证两次 lookup
/// 互不串台、互不覆盖。
#[test]
fn value_and_type_namespace_same_spelling_do_not_collide() {
    let registry = BuiltinRegistryId::from_index(0);
    let value_entity = EntityId::Builtin(BuiltinEntityId::new(
        registry,
        BuiltinSymbolId::from_index(0),
        EntityNamespace::Value,
    ));
    let type_entity = EntityId::Builtin(BuiltinEntityId::new(
        registry,
        BuiltinSymbolId::from_index(1),
        EntityNamespace::Type,
    ));

    let mut scope = LexicalScope::new(None);
    let prev_value = scope.bind(EntityNamespace::Value, "Foo".to_string(), value_entity);
    let prev_type = scope.bind(EntityNamespace::Type, "Foo".to_string(), type_entity);

    // 不同 namespace 下同 spelling 不应互相覆盖。
    assert!(prev_value.is_none(), "value 绑定不应被覆盖");
    assert!(prev_type.is_none(), "type 绑定不应覆盖 value 绑定");

    assert_eq!(
        scope.lookup_local(EntityNamespace::Value, "Foo"),
        Some(value_entity),
        "value namespace 应解析到 value entity"
    );
    assert_eq!(
        scope.lookup_local(EntityNamespace::Type, "Foo"),
        Some(type_entity),
        "type namespace 应解析到 type entity"
    );
    assert_ne!(
        value_entity, type_entity,
        "两个 namespace 的 entity 是不同实体"
    );
}

// ── P1a tests ─────────────────────────────────────────────────────────────────

/// 辅助:构造一个空环境(无别名、无 scope 名字)。
fn empty_env<'a>(
    module_roots: &'a BTreeSet<String>,
    package_aliases: &'a BTreeSet<String>,
    service_aliases: &'a BTreeSet<String>,
    value_names: &'a BTreeSet<String>,
    type_names: &'a BTreeSet<String>,
) -> ResolutionEnv<'a> {
    ResolutionEnv::new(
        module_roots,
        package_aliases,
        service_aliases,
        value_names,
        type_names,
        None,
        None,
    )
}

fn make_anchor(
    module_segments: &[&str],
    symbol: &str,
    kind: SourceDeclarationKind,
) -> SourceDeclarationAnchor {
    let publication = PublicationIdentity::with_default_epoch(
        PublicationId::parse("example.com/pkg").expect("valid publication id"),
    );
    let module_path = AnchorModulePath::new(
        module_segments
            .iter()
            .map(|segment| segment.to_string())
            .collect(),
    );
    let selector =
        SourceSelector::new(module_path, SymbolName::new(symbol.to_string()), Some(kind));
    SourceDeclarationAnchor::new(publication, selector, kind)
}

// ── case #1 ───────────────────────────────────────────────────────────────────

/// Verification case #1:裸 `root` 解析为 `ResolverRoot`,不能当 entity final result。
///
/// `resolve_dotted_path("root", Value)` 应返回 `ResolvedPath::ResolverRoot`。
/// `as_entity()` 应为 `None`。
#[test]
fn p1a_case1_root_resolves_to_resolver_root_not_entity() {
    let empty = BTreeSet::new();
    let env = empty_env(&empty, &empty, &empty, &empty, &empty);

    let result = resolve_dotted_path(&env, "root", EntityNamespace::Value)
        .expect("root 应解析成功(非 None)");

    // root 必须是 ResolverRoot,不能是 Entity 或 PathPrefix。
    assert!(
        matches!(result, ResolvedPath::ResolverRoot(_)),
        "root 应解析为 ResolverRoot,实际:{result:?}"
    );
    // 不能作为 entity final result。
    assert!(
        result.as_entity().is_none(),
        "root 不能当 entity final result"
    );
}

// ── case #2 ───────────────────────────────────────────────────────────────────

/// Verification case #2:`root.<module>.<symbol>` 在有 current publication entity table
/// 上下文时解析到 `TopLevelEntityId`。
///
/// `root` 单独解析 → `ResolverRoot`。
/// `root.user` 含点号 → `PathPrefix`(中间前缀,不是 entity)。
/// `root.user.getUser` → `Entity(TopLevel(...))`。
/// unknown symbol → 不解析为 entity。
#[test]
fn p1a_case2_root_module_symbol_resolves_to_top_level_entity() {
    let mut module_roots = BTreeSet::new();
    module_roots.insert("user".to_string());
    let empty = BTreeSet::new();
    let mut top_level_entities = PublicationEntityTable::new();
    let get_user_id = top_level_entities.push(TopLevelEntity::with_anchor(
        EntityKind::Function,
        make_anchor(&["user"], "getUser", SourceDeclarationKind::Function),
    ));
    top_level_entities.push(TopLevelEntity::with_anchor(
        EntityKind::Function,
        make_anchor(
            &["internal", "user"],
            "getUser",
            SourceDeclarationKind::Function,
        ),
    ));
    let env = empty_env(&module_roots, &empty, &empty, &empty, &empty)
        .with_current_publication_entities(&top_level_entities);

    // 裸 `root` → ResolverRoot。
    let root_result =
        resolve_dotted_path(&env, "root", EntityNamespace::Value).expect("root 应解析成功");
    assert!(
        matches!(root_result, ResolvedPath::ResolverRoot(_)),
        "裸 root 应为 ResolverRoot"
    );

    // `root.user` → PathPrefix(中间态)。
    let prefix_result = resolve_dotted_path(&env, "root.user", EntityNamespace::Value)
        .expect("root.user 应解析成功");
    assert!(
        matches!(prefix_result, ResolvedPath::PathPrefix(_)),
        "root.user 中间前缀应为 PathPrefix,实际:{prefix_result:?}"
    );
    assert!(
        prefix_result.as_entity().is_none(),
        "root.user 中间前缀不能当 entity final result"
    );

    // `root.<module>.<symbol>` → TopLevel entity。
    let entity_result = resolve_dotted_path(&env, "root.user.getUser", EntityNamespace::Value)
        .expect("root.user.getUser 应解析到顶层 source entity");
    assert_eq!(
        entity_result.as_entity(),
        Some(EntityId::TopLevel(get_user_id)),
        "完整 source selector 应解析到 TopLevelEntityId"
    );

    // source selector 不应退化成 public/display path:存在 internal.user.getUser 时,
    // root.user.getUser 仍只匹配 source module `user` 的声明。
    assert_ne!(
        top_level_entities
            .resolve_source_selector("internal.user", "getUser", EntityNamespace::Value)
            .map(EntityId::TopLevel),
        entity_result.as_entity(),
        "root lookup 必须按完整 source module path + symbol 匹配"
    );

    let unknown_result = resolve_dotted_path(&env, "root.user.missing", EntityNamespace::Value);
    assert!(
        unknown_result
            .as_ref()
            .and_then(ResolvedPath::as_entity)
            .is_none(),
        "unknown symbol 不能作为 entity final result"
    );
}

#[test]
fn p1a_root_current_publication_module_prefix_remains_resolved_prefix() {
    let empty = BTreeSet::new();
    let mut top_level_entities = PublicationEntityTable::new();
    top_level_entities.push(TopLevelEntity::with_anchor(
        EntityKind::Function,
        make_anchor(
            &["internal", "example"],
            "run",
            SourceDeclarationKind::Function,
        ),
    ));
    let env = empty_env(&empty, &empty, &empty, &empty, &empty)
        .with_current_publication_entities(&top_level_entities);

    let prefix = resolve_dotted_path(&env, "root.internal.example", EntityNamespace::Value)
        .expect("known current-publication module path should resolve as a prefix");
    assert!(
        matches!(prefix, ResolvedPath::PathPrefix(_)),
        "known module path should be a prefix, not an unresolved root: {prefix:?}"
    );
    assert!(
        resolve_dotted_path(&env, "root.internal.missing", EntityNamespace::Value).is_none(),
        "unknown module path should still be rejected"
    );
}

#[test]
fn p1a_root_current_publication_nested_module_prefix_in_longer_path_is_resolved_prefix() {
    let empty = BTreeSet::new();
    let mut top_level_entities = PublicationEntityTable::new();
    top_level_entities.push(TopLevelEntity::with_anchor(
        EntityKind::Alias,
        make_anchor(
            &["api", "raw_http"],
            "MyResponse",
            SourceDeclarationKind::Alias,
        ),
    ));
    let env = empty_env(&empty, &empty, &empty, &empty, &empty)
        .with_current_publication_entities(&top_level_entities);

    let prefix = resolve_dotted_path(&env, "root.api.raw_http.MyResponse", EntityNamespace::Value)
        .expect("known nested module prefix should resolve even in value-position diagnostics");
    assert!(
        matches!(prefix, ResolvedPath::PathPrefix(_)),
        "value-position diagnostic resolver should accept known nested module prefixes: {prefix:?}"
    );
    assert!(
        resolve_dotted_path(&env, "root.user.missing", EntityNamespace::Value).is_none(),
        "single-segment module unknown symbol should not fall back to a prefix"
    );
}

#[test]
fn p1a_root_current_publication_std_stripped_selector_resolves_to_top_level_entity() {
    let empty = BTreeSet::new();
    let mut top_level_entities = PublicationEntityTable::new();
    let emit_id = top_level_entities.push(TopLevelEntity::with_anchor(
        EntityKind::Function,
        make_anchor(
            &["std", "telemetry"],
            "emit",
            SourceDeclarationKind::Function,
        ),
    ));
    let env = empty_env(&empty, &empty, &empty, &empty, &empty)
        .with_current_publication_entities(&top_level_entities);

    assert_eq!(
        resolve_dotted_path(&env, "root.telemetry.emit", EntityNamespace::Value)
            .and_then(|resolved| resolved.as_entity()),
        Some(EntityId::TopLevel(emit_id)),
        "official std source may use stripped root selector spelling"
    );
}

// ── case #3 ───────────────────────────────────────────────────────────────────

/// Verification case #3:`pkg.user` 这类 path prefix 不当 entity。
///
/// `pkg` 是 module root。`pkg.user` 解析为 `PathPrefix`,不是 entity。
/// 验证 `PathPrefix.as_entity()` 为 `None`。
#[test]
fn p1a_case3_module_prefix_path_is_not_entity() {
    let mut module_roots = BTreeSet::new();
    module_roots.insert("pkg".to_string());
    let empty = BTreeSet::new();
    let env = empty_env(&module_roots, &empty, &empty, &empty, &empty);

    let result = resolve_dotted_path(&env, "pkg.user", EntityNamespace::Value)
        .expect("pkg.user 应解析成功(module root 命中)");

    // module root 前缀不能是 entity。
    assert!(
        matches!(result, ResolvedPath::PathPrefix(_)),
        "pkg.user 应为 PathPrefix,实际:{result:?}"
    );
    assert!(
        result.as_entity().is_none(),
        "path prefix 不能当 entity final result"
    );
    // 确认 marker 是 MODULE_PREFIX_MARKER,不是 TOP_LEVEL_REF_MARKER。
    if let ResolvedPath::PathPrefix(prefix_id) = &result {
        assert_eq!(
            prefix_id.index(),
            MODULE_PREFIX_MARKER,
            "module root prefix 应使用 MODULE_PREFIX_MARKER"
        );
    }
}

// ── case #4 ───────────────────────────────────────────────────────────────────

/// Verification case #4:local / parameter 解析到 Local entity id。
///
/// `value_names` 包含 `x`、有 `local_owner` → 解析为 `Entity(Local(...))`。
/// `as_entity()` 应为 `Some`,且是 `EntityId::Local` variant。
///
/// 注意:真实的 `local_index` 值由 `stable_name_index("x")` 占位;上层 table 填入真实
/// owner-bound id 后才是最终结果。P1a 只验 kind/variant。
#[test]
fn p1a_case4_local_parameter_resolves_to_local_entity() {
    let mut value_names = BTreeSet::new();
    value_names.insert("x".to_string());
    let empty = BTreeSet::new();
    let owner = LocalEntityOwnerId::from_index(0);

    let env = ResolutionEnv::new(
        &empty, // module_roots
        &empty, // package_aliases
        &empty, // service_aliases
        &value_names,
        &empty, // type_names
        Some(owner),
        None, // type_param_owner
    );

    let result = resolve_dotted_path(&env, "x", EntityNamespace::Value).expect("x 应解析成功");

    let entity = result
        .as_entity()
        .expect("local 应解析为 entity final result");
    assert!(
        matches!(entity, EntityId::Local(_)),
        "local 应解析为 EntityId::Local,实际:{entity:?}"
    );
}

// ── case #5 ───────────────────────────────────────────────────────────────────

/// Verification case #5:type parameter 解析到 type-parameter entity(type namespace)。
///
/// `type_names` 包含 `T`、有 `type_param_owner` → type namespace 下解析为
/// `Entity(TypeParameter(...))`。
///
/// P1a 只验 kind:真实 `param_index` 由持有 table 的上层填。
#[test]
fn p1a_case5_type_parameter_resolves_to_type_parameter_entity() {
    let mut type_names = BTreeSet::new();
    type_names.insert("T".to_string());
    let empty = BTreeSet::new();
    let generic_owner = GenericOwnerId::from_index(0);

    let env = ResolutionEnv::new(
        &empty, // module_roots
        &empty, // package_aliases
        &empty, // service_aliases
        &empty, // value_names
        &type_names,
        None, // local_owner
        Some(generic_owner),
    );

    let result = resolve_dotted_path(&env, "T", EntityNamespace::Type)
        .expect("T 在 type namespace 应解析成功");

    let entity = result
        .as_entity()
        .expect("type parameter 应是 entity final result");
    assert!(
        matches!(entity, EntityId::TypeParameter(_)),
        "type parameter 应解析为 EntityId::TypeParameter,实际:{entity:?}"
    );
}

// ── case #6 ───────────────────────────────────────────────────────────────────

/// Verification case #6:package callable → ExternalPackage entity。
///
/// `package_aliases` 包含 `myPkg` → `myPkg.doThing` 解析为 `Entity(ExternalPackage(...))`。
#[test]
fn p1a_case6_package_alias_resolves_to_external_package_entity() {
    let mut package_aliases = BTreeSet::new();
    package_aliases.insert("myPkg".to_string());
    let empty = BTreeSet::new();

    let env = empty_env(&empty, &package_aliases, &empty, &empty, &empty);

    let result = resolve_dotted_path(&env, "myPkg.doThing", EntityNamespace::Value)
        .expect("myPkg.doThing 应解析成功");

    let entity = result
        .as_entity()
        .expect("package callable 应是 entity final result");
    assert!(
        matches!(entity, EntityId::ExternalPackage(_)),
        "package callable 应解析为 EntityId::ExternalPackage,实际:{entity:?}"
    );
}

// ── case #7 ───────────────────────────────────────────────────────────────────

/// Verification case #7:service operation → ExternalService entity。
///
/// `service_aliases` 包含 `mySvc` → `mySvc.doOp` 解析为 `Entity(ExternalService(...))`。
#[test]
fn p1a_case7_service_alias_resolves_to_external_service_entity() {
    let mut service_aliases = BTreeSet::new();
    service_aliases.insert("mySvc".to_string());
    let empty = BTreeSet::new();

    let env = empty_env(&empty, &empty, &service_aliases, &empty, &empty);

    let result = resolve_dotted_path(&env, "mySvc.doOp", EntityNamespace::Value)
        .expect("mySvc.doOp 应解析成功");

    let entity = result
        .as_entity()
        .expect("service operation 应是 entity final result");
    assert!(
        matches!(entity, EntityId::ExternalService(_)),
        "service operation 应解析为 EntityId::ExternalService,实际:{entity:?}"
    );
}

// ── case #8 ───────────────────────────────────────────────────────────────────

/// Verification case #8:package alias 与 service alias 同 display path **不产同 entity
/// kind**(这条最关键)。
///
/// 场景:假设 package alias 和 service alias 都叫 `dep`。若分别作为 package alias 或
/// service alias 出现,`dep.call` 应产出不同 entity kind——ExternalPackage ≠ ExternalService。
///
/// 这验证了 P1a 把两种别名拆开的核心价值(架构 case #8,L429-434)。
#[test]
fn p1a_case8_same_display_path_package_alias_vs_service_alias_different_entity_kind() {
    // package alias `dep`
    let mut package_aliases = BTreeSet::new();
    package_aliases.insert("dep".to_string());
    let empty = BTreeSet::new();

    let pkg_env = empty_env(&empty, &package_aliases, &empty, &empty, &empty);
    let pkg_result = resolve_dotted_path(&pkg_env, "dep.call", EntityNamespace::Value)
        .expect("dep.call(package) 应解析成功");
    let pkg_entity = pkg_result
        .as_entity()
        .expect("package callable 应是 entity");

    // service alias `dep`(同 display name)
    let mut service_aliases = BTreeSet::new();
    service_aliases.insert("dep".to_string());

    let svc_env = empty_env(&empty, &empty, &service_aliases, &empty, &empty);
    let svc_result = resolve_dotted_path(&svc_env, "dep.call", EntityNamespace::Value)
        .expect("dep.call(service) 应解析成功");
    let svc_entity = svc_result
        .as_entity()
        .expect("service operation 应是 entity");

    // 同 display path 但来自 package alias 和 service alias 的 entity 不能是同种类型。
    assert!(
        matches!(pkg_entity, EntityId::ExternalPackage(_)),
        "package alias 应产 ExternalPackage,实际:{pkg_entity:?}"
    );
    assert!(
        matches!(svc_entity, EntityId::ExternalService(_)),
        "service alias 应产 ExternalService,实际:{svc_entity:?}"
    );
    // 最重要:两者 EntityId 不同(一个 ExternalPackage,一个 ExternalService)。
    assert_ne!(
        std::mem::discriminant(&pkg_entity),
        std::mem::discriminant(&svc_entity),
        "package alias 与 service alias 同 display path 不能产同 entity discriminant"
    );
}

// ── case #9 ───────────────────────────────────────────────────────────────────

/// Verification case #9:literal / 匿名临时不建 source entity。
///
/// 字面量是 value,但没有名字、定义点或引用一致性——不是 entity(架构 Core Invariant)。
/// resolver 对纯字面量路径(无点号、非已知 root)应返回 `None`(未解析),不会产 entity。
///
/// 另外验证:非点号路径根本不进入 resolver 的 entity 产出链——resolver 只处理点号路径的
/// root 分类,字面量 / 匿名临时引用不在 resolver 范围内,返回 `None` 正确表示"不关注"。
///
/// 降级说明:P1a resolver 只做点号路径 root 判断,对于纯字面量 / 匿名临时这一 case 的完整
/// 语义验证(确认 lowering 不为它们分配 entity id)属于 P3b 范围。这里只能验 resolver
/// 不为无名字的"路径"产出 entity 这一层。
#[test]
fn p1a_case9_literal_anonymous_no_entity_from_resolver() {
    let empty = BTreeSet::new();
    let env = empty_env(&empty, &empty, &empty, &empty, &empty);

    // 字面量"路径"(无点号、无已知 root):resolver 返回 None,不产 entity。
    let result_literal = resolve_dotted_path(&env, "42", EntityNamespace::Value);
    assert!(
        result_literal.is_none(),
        "字面量不应产生任何 ResolvedPath(resolver 返回 None)"
    );

    // 无名字的路径(空字符串):resolver 返回 None。
    let result_empty = resolve_dotted_path(&env, "", EntityNamespace::Value);
    assert!(result_empty.is_none(), "空路径不应产生任何 ResolvedPath");

    // 匿名临时:类似 `_anon`(不在任何 alias/root/scope 中):resolver 返回 None。
    let result_anon = resolve_dotted_path(&env, "_anon.x", EntityNamespace::Value);
    assert!(
        result_anon.is_none(),
        "匿名临时引用不应产生任何 ResolvedPath(resolver 不认识 _anon root)"
    );

    // 补充:resolver 对顶层已解析名字产出 PathPrefix(TOP_LEVEL_REF_MARKER),
    //       不是 entity——这也确认了 as_entity() 为 None。
    let mut value_names = BTreeSet::new();
    value_names.insert("foo".to_string());
    let top_env = empty_env(&empty, &empty, &empty, &value_names, &empty);
    let top_result = resolve_dotted_path(&top_env, "foo", EntityNamespace::Value)
        .expect("顶层已知名字应解析成功");
    if let ResolvedPath::PathPrefix(prefix_id) = &top_result {
        assert_eq!(
            prefix_id.index(),
            TOP_LEVEL_REF_MARKER,
            "顶层已解析引用应使用 TOP_LEVEL_REF_MARKER"
        );
        assert!(
            top_result.as_entity().is_none(),
            "顶层占位 PathPrefix 不是 entity final result(须由上层 table 填)"
        );
    }
}
