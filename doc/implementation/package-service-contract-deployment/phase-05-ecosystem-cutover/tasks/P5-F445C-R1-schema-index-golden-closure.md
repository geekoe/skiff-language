# P5-F445C-R1 Schema-index golden closure

状态：Ready。独立、test-only canonical golden审计与闭合。

## 直接父节点

- `P5-F445C-package-interface-identity-normalization-result.md`

F445G执行中，`compiler/tests/builtin_canonical_spelling.rs` 的
`declared_source_aliases_emit_only_canonical_file_ir_builtin_names`先停在schema-index golden，
阻挡其后的File IR断言。此任务必须先证明漂移来源，不能让F445G顺手更新。

## 输入与审计

使用能独立编译的两个历史production点：

- identity修复后：`e48e7e11`
- identity修复前：`42edd1b5`

分别在隔离临时worktree/target运行完整限定focused test：

```bash
cargo test -p skiff-compiler --test builtin_canonical_spelling \
  declared_source_aliases_emit_only_canonical_file_ir_builtin_names -- --nocapture
```

记录两个点的actual schema-index、old golden以及是否由F445C引起。临时worktree和target结束时清理。

## 实现边界

仅当 current canonical projection稳定证明实际值为：

```text
skiff-package-schema-index-v1:sha256:26b7640548d50a600c5e04e0b61851eb66d43b34bca65c26da99bacec2a7f577
```

才可只更新：

`compiler/tests/builtin_canonical_spelling.rs`

中的对应schema-index golden。不得修改production projection、File IR golden、identity算法或timeout
实现。若前后值不稳定、F445C意外改变production artifact而父result声称invariance，停止并上报。

更新后运行focused test；由于base仍是pre-timeout语法点，该测试应完整到达并通过其既有File IR断言。
再运行：

```bash
cargo fmt --check
git diff --check
```

## worktree与提交

worktree：

`/Users/geek/workspace/skiff-p5-f445c-r1-schema-golden`

branch：

`codex/p5-f445c-r1-schema-golden`

base：`e48e7e11`，再cherry-pick本任务文档。

提交test-only implementation，再只新增并提交：

`P5-F445C-R1-schema-index-golden-closure-result.md`

最终clean。不得派子Agent、merge/rebase/push、stable/live/network。
