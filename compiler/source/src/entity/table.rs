//! Owner-local entity tables(架构 `Publication Entity Table` 节 L164-243、
//! `Local Entity Tables` 节 L316-340)。
//!
//! 这些 table 是 owner context 的物理载体,也是 id 唯一的合法构造点:`push_*` 返回带 owner
//! 绑定的 newtype id(见 `id.rs`),外部拿不到"凭空造一个 index"的入口。这就是架构 L217-225
//! "裸 `3` 不是 stable identity" 的实现手段。
//!
//! P0 只立骨架:`push_*` 立住签名与 owner 绑定,真正存什么 entity payload(完整 `LocalEntity`
//! 内容、`LexicalScope` 名字表的填充)随 P1a typed resolution 落地。

use std::collections::BTreeMap;

use super::anchor::{SourceDeclarationAnchor, SourceDeclarationKind};
use super::id::EntityId;
use super::id::{
    GenericOwnerId, LocalEntityId, LocalEntityOwnerId, TopLevelEntityId, TypeParameterEntityId,
};
use super::kind::{EntityKind, EntityNamespace};
use super::resolution::ResolverRoot;

/// package vs service 在 source compile 阶段共享同一种 entity table;差异在 projection /
/// linkage policy,不在 table(架构 L228-243)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationKind {
    Package,
    Service,
}

/// 一个 publication 的 entity model 顶层容器(架构 L237-243)。
#[derive(Debug)]
pub struct PublicationEntityModel {
    kind: PublicationKind,
    top_level: PublicationEntityTable,
    module_index: ModulePathIndex,
    resolver_roots: ResolverRootTable,
    local_tables: Vec<LocalEntityTable>,
}

impl PublicationEntityModel {
    pub fn new(kind: PublicationKind) -> Self {
        Self {
            kind,
            top_level: PublicationEntityTable::new(),
            module_index: ModulePathIndex::new(),
            resolver_roots: ResolverRootTable::new(),
            local_tables: Vec::new(),
        }
    }

    pub fn from_declaration_anchors(
        kind: PublicationKind,
        anchors: &[SourceDeclarationAnchor],
    ) -> Self {
        let mut model = Self::new(kind);
        for anchor in anchors {
            model.top_level.push(TopLevelEntity::with_anchor(
                entity_kind_for_source_declaration_kind(anchor.kind()),
                anchor.clone(),
            ));
        }
        model
    }

    pub fn kind(&self) -> PublicationKind {
        self.kind
    }

    pub fn top_level(&self) -> &PublicationEntityTable {
        &self.top_level
    }

    pub fn top_level_mut(&mut self) -> &mut PublicationEntityTable {
        &mut self.top_level
    }
}

/// 顶层声明表(架构 L166-175,L246)。覆盖当前 production source set 中所有顶层声明。
///
/// **id 唯一合法构造点**:`push` 返回的 `TopLevelEntityId` 的 index 私有(见 `id.rs`),只能
/// 由这里按插入顺序分配。
#[derive(Debug)]
pub struct PublicationEntityTable {
    entities: Vec<TopLevelEntity>,
}

impl PublicationEntityTable {
    pub fn new() -> Self {
        Self {
            entities: Vec::new(),
        }
    }

    /// 登记一个顶层 entity,返回 owner-bound id。
    ///
    /// 注意:这个 index 是 owner-local handle,**不是** declaration anchor;若顶层声明进入
    /// ABI / schema closure,必须另投影到 `SourceDeclarationAnchor`(架构 L177-181)。
    pub fn push(&mut self, entity: TopLevelEntity) -> TopLevelEntityId {
        let index = self.entities.len() as u32;
        self.entities.push(entity);
        TopLevelEntityId::from_owner_index(index)
    }

    pub fn get(&self, id: TopLevelEntityId) -> Option<&TopLevelEntity> {
        self.entities.get(id.owner_local_index() as usize)
    }

    /// Resolve a current-publication source selector to its owner-local top-level entity id.
    ///
    /// This is intentionally keyed by the declaration anchor's source module path + source symbol,
    /// not by public/export path or display string. `root.<module>.<symbol>` lookup must land on a
    /// source declaration entity only when the full selector is present in this owner table.
    pub fn resolve_source_selector(
        &self,
        module_path: &str,
        symbol: &str,
        namespace: EntityNamespace,
    ) -> Option<TopLevelEntityId> {
        self.entities
            .iter()
            .enumerate()
            .find_map(|(index, entity)| {
                if top_level_namespace(entity.kind())? != namespace {
                    return None;
                }
                let declaration_kind = source_declaration_kind_for_entity_kind(entity.kind())?;
                let anchor = entity.anchor()?;
                if anchor_matches_source_key(anchor, module_path, symbol, declaration_kind) {
                    return Some(TopLevelEntityId::from_owner_index(index as u32));
                }
                None
            })
    }

    pub fn contains_source_module_path(&self, module_path: &str) -> bool {
        self.entities.iter().any(|entity| {
            entity
                .anchor()
                .is_some_and(|anchor| anchor_matches_source_module_path(anchor, module_path))
        })
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }
}

fn anchor_matches_source_key(
    anchor: &SourceDeclarationAnchor,
    module_path: &str,
    symbol: &str,
    kind: SourceDeclarationKind,
) -> bool {
    anchor.matches_source_key(module_path, symbol, kind)
        || std_root_module_candidate(module_path).is_some_and(|std_module_path| {
            anchor.matches_source_key(&std_module_path, symbol, kind)
        })
}

fn anchor_matches_source_module_path(anchor: &SourceDeclarationAnchor, module_path: &str) -> bool {
    anchor.matches_source_module_path(module_path)
        || std_root_module_candidate(module_path)
            .is_some_and(|std_module_path| anchor.matches_source_module_path(&std_module_path))
}

fn std_root_module_candidate(module_path: &str) -> Option<String> {
    if module_path.starts_with("__private")
        && (module_path.len() == "__private".len()
            || module_path.as_bytes().get("__private".len()) == Some(&b'.'))
    {
        return None;
    }
    (!module_path.starts_with("std.")).then(|| format!("std.{module_path}"))
}

fn top_level_namespace(kind: EntityKind) -> Option<EntityNamespace> {
    match kind {
        EntityKind::Type | EntityKind::Alias | EntityKind::Interface | EntityKind::DbObject => {
            Some(EntityNamespace::Type)
        }
        EntityKind::Function | EntityKind::Const => Some(EntityNamespace::Value),
        EntityKind::ImplMethod
        | EntityKind::Local
        | EntityKind::Parameter
        | EntityKind::PatternBinding
        | EntityKind::TypeParameter
        | EntityKind::PackageCapability
        | EntityKind::ExternalPackageSymbol
        | EntityKind::ExternalServiceOperation
        | EntityKind::ExternalServiceInstance
        | EntityKind::BuiltinSymbol => None,
    }
}

fn source_declaration_kind_for_entity_kind(kind: EntityKind) -> Option<SourceDeclarationKind> {
    match kind {
        EntityKind::Type => Some(SourceDeclarationKind::Type),
        EntityKind::Alias => Some(SourceDeclarationKind::Alias),
        EntityKind::Interface => Some(SourceDeclarationKind::Interface),
        EntityKind::Function => Some(SourceDeclarationKind::Function),
        EntityKind::Const => Some(SourceDeclarationKind::Const),
        EntityKind::DbObject => Some(SourceDeclarationKind::DbObject),
        EntityKind::ImplMethod
        | EntityKind::Local
        | EntityKind::Parameter
        | EntityKind::PatternBinding
        | EntityKind::TypeParameter
        | EntityKind::PackageCapability
        | EntityKind::ExternalPackageSymbol
        | EntityKind::ExternalServiceOperation
        | EntityKind::ExternalServiceInstance
        | EntityKind::BuiltinSymbol => None,
    }
}

fn entity_kind_for_source_declaration_kind(kind: SourceDeclarationKind) -> EntityKind {
    match kind {
        SourceDeclarationKind::Type => EntityKind::Type,
        SourceDeclarationKind::Alias => EntityKind::Alias,
        SourceDeclarationKind::Interface => EntityKind::Interface,
        SourceDeclarationKind::Function => EntityKind::Function,
        SourceDeclarationKind::Const => EntityKind::Const,
        SourceDeclarationKind::DbObject => EntityKind::DbObject,
    }
}

/// 顶层 entity payload(架构 L98-99)。P0 骨架:先承载 kind 与可选 declaration anchor。
#[derive(Debug, Clone)]
pub struct TopLevelEntity {
    kind: EntityKind,
    /// 进入 ABI / schema closure 的顶层声明须有结构化 anchor;尚未投影时为 `None`。
    anchor: Option<SourceDeclarationAnchor>,
}

impl TopLevelEntity {
    pub fn new(kind: EntityKind) -> Self {
        Self { kind, anchor: None }
    }

    pub fn with_anchor(kind: EntityKind, anchor: SourceDeclarationAnchor) -> Self {
        Self {
            kind,
            anchor: Some(anchor),
        }
    }

    pub fn kind(&self) -> EntityKind {
        self.kind
    }

    pub fn anchor(&self) -> Option<&SourceDeclarationAnchor> {
        self.anchor.as_ref()
    }
}

/// 局部 entity table(架构 L183-196,L316-340)。owner = 函数 / method / const initializer /
/// callback / match arm 等。局部 entity 不进顶层表、不进 public API graph、不参与 ABI
/// nominal identity 或 contract revision。
#[derive(Debug)]
pub struct LocalEntityTable {
    owner: LocalEntityOwner,
    owner_id: LocalEntityOwnerId,
    entities: Vec<LocalEntity>,
    scopes: Vec<LexicalScope>,
}

impl LocalEntityTable {
    pub fn new(owner: LocalEntityOwner, owner_id: LocalEntityOwnerId) -> Self {
        Self {
            owner,
            owner_id,
            entities: Vec::new(),
            scopes: Vec::new(),
        }
    }

    /// 登记一个局部 entity,返回绑定到本 table owner 的 id。
    pub fn push(&mut self, entity: LocalEntity) -> LocalEntityId {
        let index = self.entities.len() as u32;
        self.entities.push(entity);
        LocalEntityId::from_owner_index(self.owner_id, index)
    }

    pub fn owner(&self) -> &LocalEntityOwner {
        &self.owner
    }
}

/// 局部 entity table 的 owner 类别(架构 L316-318)。P0 骨架。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalEntityOwner {
    Function,
    ImplMethod,
    ConstInitializer,
    Callback,
    MatchArm,
}

/// 局部 entity payload(参数 / 局部变量 / pattern binding 等,架构 L192-196)。P0 骨架。
#[derive(Debug, Clone)]
pub struct LocalEntity {
    kind: EntityKind,
}

impl LocalEntity {
    pub fn new(kind: EntityKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> EntityKind {
        self.kind
    }
}

/// 词法作用域(架构 L320-330)。
///
/// **contract**:lookup key 必须包含 namespace。这里用 `(EntityNamespace, Name)` 做 key,
/// 因此同一 spelling 在 value / type namespace 并存时不丢信息——这是架构 L327-330 "不能用
/// `Map<Name, EntityId>` 表示长期模型" 的实现,也是 P0 verification 第二条 case 的地基。
#[derive(Debug, Default)]
pub struct LexicalScope {
    parent: Option<LexicalScopeId>,
    names: BTreeMap<(EntityNamespace, String), EntityId>,
}

impl LexicalScope {
    pub fn new(parent: Option<LexicalScopeId>) -> Self {
        Self {
            parent,
            names: BTreeMap::new(),
        }
    }

    /// 绑定一个名字到 entity。返回同 (namespace, name) 下被覆盖的旧绑定(若有)。
    pub fn bind(
        &mut self,
        namespace: EntityNamespace,
        name: String,
        target: EntityId,
    ) -> Option<EntityId> {
        self.names.insert((namespace, name), target)
    }

    /// 在本 scope 内查名字(不向 parent 递归;递归 lookup 留待 resolution 实现)。
    pub fn lookup_local(&self, namespace: EntityNamespace, name: &str) -> Option<EntityId> {
        self.names.get(&(namespace, name.to_string())).copied()
    }

    pub fn parent(&self) -> Option<LexicalScopeId> {
        self.parent
    }
}

/// 词法作用域 handle(owner = 某个 `LocalEntityTable`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LexicalScopeId {
    index: u32,
}

impl LexicalScopeId {
    pub fn from_index(index: u32) -> Self {
        Self { index }
    }

    pub fn index(self) -> u32 {
        self.index
    }
}

/// module path lookup 组织结构(架构 L242,L256-260)。文件 / module path 只是 lookup 组织,
/// 不是 runtime 地址,也不是 public path。P0 骨架。
#[derive(Debug, Default)]
pub struct ModulePathIndex {
    paths: Vec<ModulePath>,
}

impl ModulePathIndex {
    pub fn new() -> Self {
        Self { paths: Vec::new() }
    }

    pub fn intern(&mut self, path: ModulePath) -> u32 {
        if let Some(existing) = self.paths.iter().position(|candidate| candidate == &path) {
            return existing as u32;
        }
        let index = self.paths.len() as u32;
        self.paths.push(path);
        index
    }
}

/// module path(lookup 组织视角,分段保留)。
///
/// 与 `anchor::ModulePath`(declaration anchor 名字组成)区分;见 `anchor.rs` 的说明,P0
/// 暂不强行统一。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModulePath {
    segments: Vec<String>,
}

impl ModulePath {
    pub fn new(segments: Vec<String>) -> Self {
        Self { segments }
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

/// resolver root 注册表(架构 L242,L346-353)。P0 骨架:登记 root 并返回 handle。
#[derive(Debug, Default)]
pub struct ResolverRootTable {
    roots: Vec<ResolverRoot>,
}

impl ResolverRootTable {
    pub fn new() -> Self {
        Self { roots: Vec::new() }
    }

    pub fn push(&mut self, root: ResolverRoot) -> u32 {
        let index = self.roots.len() as u32;
        self.roots.push(root);
        index
    }

    pub fn get(&self, index: u32) -> Option<&ResolverRoot> {
        self.roots.get(index as usize)
    }
}

/// 给 generic owner 分配序号的轻量 allocator(P0 骨架),保证 `GenericOwnerId` /
/// `TypeParameterEntityId` 也经受控入口构造,不被裸 `u32` 冒充。
#[derive(Debug, Default)]
pub struct GenericOwnerAllocator {
    next: u32,
}

impl GenericOwnerAllocator {
    pub fn new() -> Self {
        Self { next: 0 }
    }

    pub fn allocate(&mut self) -> GenericOwnerId {
        let id = GenericOwnerId::from_index(self.next);
        self.next += 1;
        id
    }
}

/// 在某个 generic owner 下分配 type parameter id(架构 L198-205)。
pub fn type_parameter_id(owner: GenericOwnerId, param_index: u32) -> TypeParameterEntityId {
    TypeParameterEntityId::from_owner_index(owner, param_index)
}
