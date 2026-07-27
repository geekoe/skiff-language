# P5-F445C-R3 Provider build baseline audit

状态：Ready。只读、可编译 pre-I3 identity 基线审计。

## 直接父节点

- `P5-F445C-package-interface-identity-normalization-result.md`
- `P5-F445C-R2-package-identity-golden-closure-result.md`

F445G 把 File IR 持久格式从 v8/v6/v1 升级到 v9/v7/v2 后，
`package_interface_identity::direct_package_interface_identity_normalizes_dependency_and_package_id_owners`
观测到 provider package build 从测试常量 `3b9f3647…116ef` 变为
`565fb88e…35bb8`。在 F445G 更新该版本相关 golden 前，本 leaf 只证明 pre-I3 的可编译基线。

## 输入与命令

使用已经包含 F445C 与 test-only R1/R2 golden 闭合、但不包含 F445D/F445G 的可编译 worktree：

`/Users/geek/workspace/skiff-p5-f445c-r2-package-goldens`

使用独立 target 运行：

```bash
cargo test -p skiff-compiler --test package_interface_identity \
  direct_package_interface_identity_normalizes_dependency_and_package_id_owners -- --nocapture
```

记录：

- provider package build identity；
- provider Local ABI identity；
- publication receipt 是否仍等于 provider build；
- focused test verdict。

不得修改测试常量或 production。若测试失败，只记录 expected/actual 与最早失败位置。

## 输出

只新增并提交：

`P5-F445C-R3-provider-build-baseline-audit-result.md`

不得修改其它文件、派子 Agent、merge/rebase/push、stable/live/network。最终 clean。
