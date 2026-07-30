# P5-F198：JsonObject.has Callable Semantics 结果

状态：Implementation complete；真实 Account ecosystem acceptance 等待 official package state 收口

## 直接父任务

- `P5-F198-json-object-has-callable-semantics.md`

## 实现结果

- 在精确 callable semantics registry 中登记
  `receiver:JsonObject.has@1`。
- 该 binding 的 compiler facts 为：
  - receiver 与 field 参数只读；
  - 不修改 caller-reachable value；
  - 不返回或抛出 caller alias；
  - 不逃逸 caller value；
  - 不要求 same-heap identity；
  - 不调用未知目标；
  - 不挂起；
  - 返回 fresh `bool`。
- 只为 `JsonObject.has` 增加一个 `string` field 的精确参数检查。错误 receiver、缺少参数、
  多余参数和错误 field 类型均 fail closed。
- 未给 `JsonObject.get`、`JsonObject.length`、`JsonObject.delete`、`JsonObject.clone` 或其他
  receiver callable 增加 semantics；没有按 JsonObject family 泛化。

## 验证

- `cargo test -p skiff-artifact-model callable_semantics_registry_is_sparse_exact_and_safe`
- `cargo test -p skiff-compiler-source exact_json_object_has_target_is_read_only_detached_and_non_suspending`
- `cargo test -p skiff-compiler json_object_has_enforces_exact_receiver_and_arity`
- `cargo check --workspace`
- `git diff --check`

以上验证通过。聚焦 fixture 覆盖 Account 形状的
`verifyDomainChallenge -> jsonObjectField -> value.has(field)` source callable facts、精确
resolved target、fresh bool provenance 和 File IR receiver builtin lowering。

## 真实 Account 验证状态

使用只读 stable artifact store 的 hard-link 隔离副本执行真实 Account build，compiler 在进入
Account 源码编译前按预期 fail closed：

```text
package dependency skiff.run/http-session@1.0.0 has no published PackageArtifact pointer
```

该 store 仍只有旧 assembly package 记录，没有当前 compiler 要求的 official package pointer；
生产 CLI 也会正确拒绝把仓库内保留的 `skiff.run/std` 当普通第三方 package 发布。本任务没有修改
stable store，也没有绕过 package identity 规则。隔离 artifact root 已删除。

official package state 合入后，应在同一 integration HEAD 重跑真实 Account artifact 与
service-api receipt，并确认当前 receipt 声明的 21 个 operation 全部 Available（任务原文中的
“20 operations”已落后于 Account 当前 21 个公开 operation）。其中
`account.verifyDomainChallenge -> jsonObjectField -> receiver:JsonObject.has@1` 是必验路径。
