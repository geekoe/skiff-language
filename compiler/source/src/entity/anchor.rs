//! Source selector 与 declaration anchor(架构 `Publication Entity Table` 节 L246-313、
//! `Declaration Anchor And Descriptor` 节 L538-567)。
//!
//! Declaration anchor 是 nominal / API declaration 的稳定身份锚点。第一版用
//! `PublicationIdentity + SourceSelector + kind` 组成(架构 L288-297)。它必须保留完整
//! module path + source symbol name + declaration kind;**不能退化成 declaration 在文件 /
//! module / table 中的序号**(架构 L264-265,L590)。
//!
//! 这条规则使下列成立(架构 L301-304),也是 P1b verification 的地基:
//! - 同一 module / file 内重排 declaration,不改 anchor;
//! - 加 / 删无关 sibling declaration,不改既有 anchor;
//! - 把 nominal type 移到不同 module / file(即使 public path 与 descriptor 不变)得到不同
//!   anchor,视为另一个 declaration。
//!
//! P0 只立类型;`source_identity.rs` 的源码文本拼接替换归 P1b,不在本阶段触碰。

use crate::shared::id::PublicationId;

/// ABI epoch(架构 L549-557)。默认 `0`,只有开发者 / registry policy 明确要求切断 nominal
/// lineage 时才递增。普通 publication version / source hash / build id 不进入它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AbiEpoch(u32);

impl AbiEpoch {
    /// nominal lineage 默认 epoch。
    pub const ZERO: AbiEpoch = AbiEpoch(0);

    pub(super) fn from_raw(epoch: u32) -> Self {
        Self(epoch)
    }

    pub(super) fn value(self) -> u32 {
        self.0
    }
}

/// ABI owner identity(架构 L552-558)。
///
/// 它是 declaration anchor 的 owner 部分,**不是 build id**:固定由 stable publication id
/// 与显式 ABI epoch 组成。普通 publication version / source hash / build id 不进入这里。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicationIdentity {
    id: PublicationId,
    abi_epoch: AbiEpoch,
}

impl PublicationIdentity {
    pub fn new(id: PublicationId, abi_epoch: AbiEpoch) -> Self {
        Self { id, abi_epoch }
    }

    /// 默认 epoch(`0`)下的 publication identity。
    pub fn with_default_epoch(id: PublicationId) -> Self {
        Self {
            id,
            abi_epoch: AbiEpoch::ZERO,
        }
    }

    pub(super) fn id(&self) -> &PublicationId {
        &self.id
    }

    pub(super) fn abi_epoch(&self) -> AbiEpoch {
        self.abi_epoch
    }
}

/// 顶层 source declaration 的种类(架构 L274-282)。
///
/// 注意:`impl` method 不在此列——它不进入顶层 source selector,因此不独立持有 declaration
/// anchor(架构 L309)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceDeclarationKind {
    Type,
    Alias,
    Interface,
    Function,
    Const,
    DbObject,
}

/// source-layer lookup key,也是第一版顶层 source declaration anchor 的名字部分
/// (架构 L267-282)。
///
/// 典型形态 `module.path.Symbol`。它不携带 publication id,不是外部源码可写名字,也不是 ABI
/// nominal identity。作为 anchor 组成时必须保留完整 module path + symbol + kind hint,**不能
/// 替换成 declaration ordinal**。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceSelector {
    module_path: ModulePath,
    symbol: SymbolName,
    kind_hint: Option<SourceDeclarationKind>,
}

impl SourceSelector {
    pub fn new(
        module_path: ModulePath,
        symbol: SymbolName,
        kind_hint: Option<SourceDeclarationKind>,
    ) -> Self {
        Self {
            module_path,
            symbol,
            kind_hint,
        }
    }

    pub(super) fn module_path(&self) -> &ModulePath {
        &self.module_path
    }

    pub(super) fn symbol(&self) -> &SymbolName {
        &self.symbol
    }

    pub(super) fn kind_hint(&self) -> Option<SourceDeclarationKind> {
        self.kind_hint
    }
}

/// source selector 中的 module path 部分(分段保留)。
///
/// 与 `table::ModulePath`(lookup 组织视角)区分:这里是 anchor 名字组成。P0 暂各立各的,
/// 后续若证明应统一,在 P1b 收敛——届时已有真实消费方决定该共享哪种表示。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModulePath {
    segments: Vec<String>,
}

impl ModulePath {
    pub fn new(segments: Vec<String>) -> Self {
        Self { segments }
    }

    pub(super) fn segments(&self) -> &[String] {
        &self.segments
    }
}

/// source symbol 名字(P0 骨架)。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolName(String);

impl SymbolName {
    pub fn new(name: String) -> Self {
        Self(name)
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// 第一版 source declaration anchor(架构 L291-297)= publication identity + 结构化 selector +
/// declaration kind。
///
/// **稳定性契约**(架构 L301-304):derive 的 `PartialEq`/`Eq`/`Hash` 不看 declaration 在源码
/// 中的位置序号,只看 publication identity + module path + symbol + kind,因此重排 / 加 sibling
/// 不改 anchor,move-file 改 anchor。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceDeclarationAnchor {
    publication: PublicationIdentity,
    selector: SourceSelector,
    kind: SourceDeclarationKind,
}

impl SourceDeclarationAnchor {
    pub fn new(
        publication: PublicationIdentity,
        selector: SourceSelector,
        kind: SourceDeclarationKind,
    ) -> Self {
        Self {
            publication,
            selector,
            kind,
        }
    }

    pub(super) fn publication(&self) -> &PublicationIdentity {
        &self.publication
    }

    pub(super) fn selector(&self) -> &SourceSelector {
        &self.selector
    }

    pub(super) fn kind(&self) -> SourceDeclarationKind {
        self.kind
    }

    /// 检查本 anchor 是否对应给定的 (module_path_str, symbol_str, kind)。
    ///
    /// 供 contract/index.rs 在 `declaration_anchors()` 表中查找匹配 anchor 时调用,
    /// 避免暴露 `publication()`/`selector()` 的 `pub(super)` 细节。
    pub fn matches_source_key(
        &self,
        module_path_str: &str,
        symbol_str: &str,
        kind: SourceDeclarationKind,
    ) -> bool {
        self.kind == kind
            && self.selector.symbol.as_str() == symbol_str
            && self.selector.module_path.segments.join(".") == module_path_str
    }

    pub fn matches_source_module_path(&self, module_path_str: &str) -> bool {
        self.selector.module_path.segments.join(".") == module_path_str
    }
}
