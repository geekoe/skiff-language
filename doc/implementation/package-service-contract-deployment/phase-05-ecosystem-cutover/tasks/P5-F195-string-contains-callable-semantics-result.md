# P5-F195：String.contains Callable Semantics 结果

状态：Implementation complete；真实 Account ecosystem acceptance 等待 F188 official package state 收口

## 直接父任务

- `P5-F195-string-contains-callable-semantics.md`

## 实现结果

- 在精确 callable semantics registry 中登记
  `receiver:string.contains@1`。
- 该 binding 的 compiler facts 为：
  - receiver 与 needle 只读；
  - 不修改 caller-reachable value；
  - 不返回或抛出 caller alias；
  - 不逃逸 caller value；
  - 不要求 same-heap identity；
  - 不调用未知目标；
  - 不挂起；
  - 返回 fresh `bool`。
- 只为 `string.contains` 增加一个 `string` needle 的精确参数检查。错误 receiver、缺少参数和
  多余参数均 fail closed。
- 未给 `string.replaceAll` 或其他尚未审核的 receiver callable 增加 semantics；没有按 string
  family 泛化。

## 验证

- `cargo test -p skiff-artifact-model callable_semantics_registry_is_sparse_exact_and_safe`
- `cargo test -p skiff-compiler-source exact_string_contains_target_is_read_only_detached_and_non_suspending`
- `cargo test -p skiff-compiler string_contains_enforces_exact_receiver_and_arity`
- `cargo check --workspace`
- `git diff --check`

以上验证通过。聚焦 fixture 覆盖 Account `validEmail(value) -> value.contains("@")` 的 source
callable facts、精确 resolved target 和 File IR receiver builtin lowering。

## 真实 Account 验证状态

隔离 artifact root 中的真实 Account build 在进入 Account 编译前被 official package state
阻塞：当前 canonical store 尚无 `skiff.run/http-session@1.0.0` 的新
`PackageArtifact` pointer；直接重建 http-session 又要求同一 compile graph 中存在 canonical
`skiff.run/std` artifact。这个依赖状态由 F188 统一收口。

本任务未修改 stable store；用于只读诊断的临时隔离 artifact root 已删除，也未为了绕过该
blocker 扩大实现范围。F188 合入后应使用同一 integration HEAD 重跑真实 Account
artifact/contract；`register -> validEmail ->
receiver:string.contains@1` 是该轮 ecosystem acceptance 的必验路径。
