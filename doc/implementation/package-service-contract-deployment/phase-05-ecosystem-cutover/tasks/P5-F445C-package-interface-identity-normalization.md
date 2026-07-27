# P5-F445C Package interface identity normalization

状态：Ready。Skiff implementation leaf。

## 直接父节点

- `P5-F445A-package-interface-identity-normalization-preflight-result.md`

## 输入

Skiff integration：

`/Users/geek/workspace/skiff-phase-05-integration` @ `6e5d77fe`

必须 clean。

## 实现

严格按父 result 的单一路径：

1. 先在 `compiler/tests/package_interface_identity.rs` 建立 direct 与 transitive package
   fixtures，证明修复前同 package/symbol/ABI 的
   `Dependency` / `PackageId` embedded `AnyInterface.interface_abi_id`被误拒绝。
2. 只修改
   `compiler/source/src/type_resolution_model.rs::TypeResolutionModel::canonicalize_type_ref`
   的 `AnyInterface` owner：
   - decode embedded identity为 `TypeRefIr`；
   - 递归调用现有 canonicalization；
   - 用 canonical identity和canonical type args重建interface instantiation id；
   - malformed/未绑定/ABI不符继续fail closed。
3. 不改变 dependency-local rehydration，不修改 artifact projection、linker/runtime或Internals
   package source。
4. 覆盖父 result矩阵中的 direct/transitive、nullable/array/record、dependency-owned interface和
   package/symbol/ABI/generic args负例。
5. 证明 provider artifact/local ABI/receipt identity不变；若修复泄漏到projection/publication，
   停止并返回 `TASK_SCOPE_EXPANDED`。

## 写集

只允许：

- `compiler/source/src/type_resolution_model.rs`
- `compiler/tests/package_interface_identity.rs`
- `compiler/Cargo.toml`
- 本任务 result

若 RED 无法在该范围转绿，必须停止，不得加入cast或放宽exact identity。

## 验证

运行父 result列出的4条聚焦命令，并补：

```bash
cargo fmt --check
git diff --check
```

共享 target：

`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`

记录真实RED、每项test计数、负例和identity invariance。

## 提交

worktree：

`/Users/geek/workspace/skiff-p5-f445c-interface-identity`

branch：

`codex/p5-f445c-interface-identity`

先提交 implementation，再只新增并提交：

`P5-F445C-package-interface-identity-normalization-result.md`

最终clean。不得派子Agent、merge/rebase/push、stable/live/network。
