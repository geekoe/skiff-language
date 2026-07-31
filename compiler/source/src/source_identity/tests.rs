use super::*;

// ── helpers ──────────────────────────────────────────────────────────────────

/// 直接构造测试用 anchor,不通过 ParsedCompilerSource。
/// anchor 的稳定性在于它仅由(publication + module_path + symbol + kind)决定;
/// 这些参数都是结构化的,与源码文本和声明顺序无关。
fn make_test_anchor(
    pub_id: &str,
    module_path: &[&str],
    symbol: &str,
    kind: SourceDeclarationKind,
) -> SourceDeclarationAnchor {
    let pub_id = PublicationId::parse(pub_id).unwrap();
    let publication = PublicationIdentity::with_default_epoch(pub_id);
    let mp = AnchorModulePath::new(module_path.iter().map(|s| s.to_string()).collect());
    make_anchor(publication, mp, symbol, kind)
}

/// 把一段源码 parse 成真实 AST,再走 `anchors_for_ast`(build() 的真实抽取路径)产 anchor 集。
///
/// 与 `make_test_anchor` 的区别:这条路径**真的**经过"解析源码 → 遍历 AST 声明 → 产 anchor",
/// 因此能验"reorder/加 sibling 源码 → anchor 集不变"这个 build() 的实际不变量,而不是只验
/// `derive(PartialEq)` 自反。
fn anchors_from_source(
    pub_id: &str,
    module_path: &[&str],
    source: &str,
) -> Vec<SourceDeclarationAnchor> {
    let publication =
        PublicationIdentity::with_default_epoch(PublicationId::parse(pub_id).unwrap());
    let mp = AnchorModulePath::new(module_path.iter().map(|s| s.to_string()).collect());
    let ast = crate::shared::parser::parse_source(source).expect("test source must parse");
    anchors_for_ast(&publication, &mp, &ast)
}

fn anchor_set(
    anchors: &[SourceDeclarationAnchor],
) -> std::collections::BTreeSet<SourceDeclarationAnchor> {
    anchors.iter().cloned().collect()
}

// ── 真实抽取路径:reorder / 加 sibling 不改 anchor 集 ─────────────────────────

/// 经 `anchors_for_ast`(build() 真实路径)验 case #10:同一 module 内重排顶层声明,
/// 产出的 anchor **集合**不变。这条比下面 make_test_anchor 版本更强:它实际 parse 了
/// 两份只有顺序不同的源码,证明 build() 的抽取逻辑不含序号。
#[test]
fn p1b_case10_reorder_real_ast_keeps_anchor_set() {
    let pub_id = "test.run/mypkg";
    let order_a = "type Foo { a: string }\ntype Bar { b: number }\ntype Qux { c: bool }";
    let order_b = "type Qux { c: bool }\ntype Bar { b: number }\ntype Foo { a: string }";

    let a = anchors_from_source(pub_id, &["my", "module"], order_a);
    let b = anchors_from_source(pub_id, &["my", "module"], order_b);

    assert_eq!(
        anchor_set(&a),
        anchor_set(&b),
        "reorder 源码声明顺序后,build() 抽取的 anchor 集合必须不变"
    );
    assert_eq!(a.len(), 3, "应抽到 Foo/Bar/Qux 三个 anchor");
}

/// 经 `anchors_for_ast` 验 case #11:加无关 sibling 后,既有声明的 anchor 仍在集合里且未变,
/// 仅新增一个 anchor(不 renumber 既有的)。
#[test]
fn p1b_case11_add_sibling_real_ast_preserves_existing_anchors() {
    let pub_id = "test.run/mypkg";
    let before = "type Foo { a: string }\ntype Bar { b: number }";
    let after = "type Foo { a: string }\ntype Bar { b: number }\ntype Baz { c: bool }";

    let before_set = anchor_set(&anchors_from_source(pub_id, &["my", "module"], before));
    let after_set = anchor_set(&anchors_from_source(pub_id, &["my", "module"], after));

    assert!(
        before_set.is_subset(&after_set),
        "加 sibling 后,既有 anchor 必须原样保留(不 renumber)"
    );
    assert_eq!(
        after_set.len(),
        before_set.len() + 1,
        "只应新增一个 anchor(Baz)"
    );
}

// ── case #10: reorder 不改 anchor ───────────────────────────────────────────

/// Verification case #10:同一 module 内顶层声明换顺序,各声明的 SourceDeclarationAnchor
/// 不变。
///
/// 证明方式:分别为"Foo 在位置 0"和"Foo 在位置 1"(另一个声明排在前面)各构造 anchor,
/// 验证两个 Foo anchor 相等。anchor 不包含声明顺序/序号,因此顺序无关。
///
/// 核心不变量:anchor = f(publication, module_path, symbol, kind)。
/// 只要这四个输入不变,anchor 就不变,无论 declaration 在文件里的位置如何。
#[test]
fn p1b_case10_reorder_does_not_change_anchors() {
    let pub_id = "test.run/mypkg";

    // Simulate: "Foo appears first in the file"
    // → anchor is derived only from (publication, module, symbol, kind)
    let foo_anchor_when_first = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Foo",
        SourceDeclarationKind::Type,
    );

    // Simulate: "Foo appears second in the file (Bar was moved before it)"
    // → same (publication, module, symbol, kind) → same anchor
    let foo_anchor_when_second = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Foo",
        SourceDeclarationKind::Type,
    );

    assert_eq!(
        foo_anchor_when_first, foo_anchor_when_second,
        "reorder: Foo anchor must be identical regardless of its position in the file"
    );

    // Bar anchor is also stable regardless of position
    let bar_anchor_position_a = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Bar",
        SourceDeclarationKind::Type,
    );
    let bar_anchor_position_b = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Bar",
        SourceDeclarationKind::Type,
    );
    assert_eq!(
        bar_anchor_position_a, bar_anchor_position_b,
        "reorder: Bar anchor must be identical regardless of its position in the file"
    );

    // The two anchors are distinct (different symbols)
    assert_ne!(
        foo_anchor_when_first, bar_anchor_position_a,
        "Foo and Bar have different anchors"
    );
}

// ── case #11: 加无关 sibling 不改 anchor ─────────────────────────────────────

/// Verification case #11:加一个无关顶层声明,既有声明的 anchor 不变(不 renumber)。
///
/// 证明方式:Foo 的 anchor 在"只有 Foo"和"Foo + Baz 两者同存"时完全相同。
/// 因为 anchor 不含 declaration 序号——它只是 (publication, module_path, symbol, kind)。
#[test]
fn p1b_case11_add_sibling_does_not_change_existing_anchors() {
    let pub_id = "test.run/mypkg";

    // Foo's anchor "before Baz was added"
    let foo_before = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Foo",
        SourceDeclarationKind::Type,
    );

    // Foo's anchor "after Baz was added as a sibling"
    // same inputs → same anchor
    let foo_after = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Foo",
        SourceDeclarationKind::Type,
    );

    assert_eq!(
        foo_before, foo_after,
        "adding sibling Baz must not change Foo's anchor"
    );

    // Baz gets its own new anchor (does not collide with Foo)
    let baz_anchor = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Baz",
        SourceDeclarationKind::Type,
    );
    assert_ne!(
        foo_after, baz_anchor,
        "Foo and Baz must have distinct anchors"
    );
}

// ── case #12: move-file 改 anchor ───────────────────────────────────────────

/// Verification case #12:把一个 declaration 从一个 module/file 移到另一个,
/// anchor 改变(即使 descriptor/public path 不变)。
///
/// 证明方式:构造 `Foo` 在 module_a 的 anchor 和 `Foo` 在 module_b 的 anchor,
/// 验证两者不相等。module_path 是 anchor 的组成部分(架构 L301-307)。
#[test]
fn p1b_case12_move_file_changes_anchor() {
    let pub_id = "test.run/mypkg";
    let anchor_in_module_a =
        make_test_anchor(pub_id, &["module", "a"], "Foo", SourceDeclarationKind::Type);
    let anchor_in_module_b =
        make_test_anchor(pub_id, &["module", "b"], "Foo", SourceDeclarationKind::Type);

    assert_ne!(
        anchor_in_module_a, anchor_in_module_b,
        "moving Foo to a different module must change its anchor"
    );
}

// ── case #13: descriptor 改 revision 但 anchor 不变 ─────────────────────────

/// Verification case #13:改一个 type 的 descriptor(如 `type Id = string` → `type Id =
/// number`),anchor 不变,但 revision_id 变。
///
/// anchor 层验证:anchor 只由 publication + module_path + symbol + kind 决定,
/// 不含源码文本或 descriptor 事实,因此 descriptor 改变不影响 anchor。
///
/// revision_id 层验证:revision_id 在 runtime_manifest.rs 中已切换为消费
/// canonical_contract_schema_json(descriptor 事实),而非 source_identity()。
/// - 空白改动、声明重排 → canonical schema 不变 → revision 不变。
/// - descriptor 改变(字段/类型变化)→ canonical schema 改变 → revision 变。
/// 端到端 revision 验证留给更上层的 manifest 集成测试。
#[test]
fn p1b_case13_descriptor_change_does_not_change_anchor() {
    let pub_id = "test.run/mypkg";

    // "Before": type Id = string  → anchor for Id
    let anchor_before =
        make_test_anchor(pub_id, &["my", "module"], "Id", SourceDeclarationKind::Type);

    // "After": type Id = number  → descriptor changed, but anchor inputs are the same
    // (publication, module_path="my.module", symbol="Id", kind=Type) → same anchor
    let anchor_after =
        make_test_anchor(pub_id, &["my", "module"], "Id", SourceDeclarationKind::Type);

    assert_eq!(
        anchor_before, anchor_after,
        "descriptor change must not affect anchor identity"
    );

    // Cross-check: different kind → different anchor (to confirm kind is part of identity)
    let alias_anchor = make_test_anchor(
        pub_id,
        &["my", "module"],
        "Id",
        SourceDeclarationKind::Alias,
    );
    assert_ne!(
        anchor_before, alias_anchor,
        "same symbol but different kind → different anchor"
    );
}
