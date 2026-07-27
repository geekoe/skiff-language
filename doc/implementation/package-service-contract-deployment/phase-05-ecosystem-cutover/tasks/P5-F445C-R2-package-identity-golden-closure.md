# P5-F445C-R2 Package identity golden closure

状态：Ready。独立、test-only 剩余 package identity golden 审计与闭合。

## 直接父节点

- `P5-F445C-R1-schema-index-golden-closure-result.md`

R1 已证明 schema-index 漂移早于 F445C，且其修复后既有 File IR golden 通过。随后同一 focused
test 停在更后的 `CURRENT_STD_LOCAL_ABI` 旧值；`CURRENT_STD_BUILD` 断言又位于其后，因此本 leaf
一次审计这两个剩余 package identity golden，避免逐断言重复派工。

## 输入与审计

使用两个可独立编译的 production 历史点：

- F445C 前：`42edd1b5`
- F445C 后：`e48e7e11`

在两个隔离临时 worktree 中先应用 R1 的 test-only schema golden patch `784d2bff`，分别使用独立
`CARGO_TARGET_DIR` 运行：

```bash
cargo test -p skiff-compiler --test builtin_canonical_spelling \
  declared_source_aliases_emit_only_canonical_file_ir_builtin_names -- --nocapture
```

记录两个历史点的：

1. `package_local_abi.local_abi_identity` actual；
2. `package_build_id` actual。

为了到达后一断言，可在临时 worktree 中依次把前一常量改为刚观测到的 actual，但不得提交这些
临时审计改动。任务结束删除临时 worktree 与 target。

## 判定与实现边界

仅当两个历史 production 点对某个 identity 都产生完全相同的 actual，且 F445C 前后不变，才可
在正式 task worktree 中更新对应测试常量：

- `CURRENT_STD_LOCAL_ABI`
- `CURRENT_STD_BUILD`

已知 R1 首次观测的 Local ABI actual 为：

```text
skiff-package-local-abi-v7:sha256:4e370158a4a654c55f0e086509368ebbdf34c5bfb818d5161aca18fcb62711ac
```

唯一允许修改：

`compiler/tests/builtin_canonical_spelling.rs`

不得修改 production projection、identity 算法、schema-index、File IR、timeout 或其它 golden。
若两个历史点结果不同，或 focused test 继续暴露不属于上述两个常量的失败，停止并如实上报。

## 验证

正式更新后运行：

```bash
cargo test -p skiff-compiler --test builtin_canonical_spelling \
  declared_source_aliases_emit_only_canonical_file_ir_builtin_names -- --nocapture
cargo fmt --check
git diff --check
```

focused test 必须完整通过。

## worktree 与提交

worktree：

`/Users/geek/workspace/skiff-p5-f445c-r2-package-goldens`

branch：

`codex/p5-f445c-r2-package-goldens`

执行 base 必须是可独立编译的 F445C 后 production 点 `e48e7e11`，再依次 cherry-pick：

1. R1 task 文档；
2. R1 test-only schema golden implementation；
3. R1 result；
4. 本任务文档及后续 task-doc 修订。

不得以包含 F445D syntax、但尚未包含 F445G lowering 的 integration 中间点作为正式执行 base；
该中间点会按预期产生 timeout AST 到 lowering 的非穷尽匹配，无法验证本 test-only leaf。

提交 test-only implementation，再只新增并提交：

`P5-F445C-R2-package-identity-golden-closure-result.md`

最终 clean。不得派子 Agent、merge/rebase/push、stable/live/network。
