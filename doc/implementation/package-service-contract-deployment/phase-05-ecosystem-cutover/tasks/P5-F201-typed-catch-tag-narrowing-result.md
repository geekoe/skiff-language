# P5-F201：Typed Catch Tag Narrowing Provenance 结果

状态：Completed

## 直接父任务

- `P5-F201-typed-catch-tag-narrowing.md`

## 定位

source type checker 已能把 `CatchResult<T, E>` 按 `tag` 收窄为 ok / err record，但 callable
effect/provenance transfer 仍把整个 catch container 当作一个值：

- `.tag`、`.value` 和 `.exception` 都继承 try expression 的 provenance；
- 因此 tag 比较可能虚假携带 caller heap identity；
- err branch 也会泄漏 success value provenance；
- nullable success 与 `null` 比较会被当作引用 identity 比较。

这不是 Relay source 缺少类型标注，也不应通过删除 catch、把返回值泛化为 unknown 或 consumer
改写规避。

## 实现

- callable abstract value 为 typed catch 保存独立的 success / error field provenance：
  - `tag` 是 local constant；
  - narrowed `.value` 恢复 try callee 的精确 success provenance；
  - `.exception` 是当前 heap 中 materialize 的 fresh exception envelope，不继承 success value；
  - unknown callee 的 success 仍为 unknown，不被 catch 清洗。
- catch container 本身继续保留嵌套 success 的 caller reachability；直接返回整个 container 不会虚假
  丢失 alias。
- `==` / `!=` 只有两侧都可能是 reference 时才记录 same-heap identity。nullable reference 与
  `null` 的 presence 比较不要求跨 boundary 保持对象 identity。
- source type narrowing 继续 fail closed：
  - ok branch 可读 `.value`；
  - err branch和未收窄路径读取 `.value` 均报 unknown field；
  - 正反比较、early return 和 nested catch 使用同一规则。

## 验证

- typed catch 聚焦探针：
  - ok / err；
  - `==` / `!=` 与反向比较；
  - early return；
  - 未 narrowing；
  - nested catch；
  - nullable success 与 `null`；
  - unknown dynamic callee。
- `cargo test -p skiff-compiler-source typed_catch -- --nocapture`
  - `3 passed / 0 failed`
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`
  - `238 passed / 0 failed`
- `cargo check --workspace`
  - 通过
- `git diff --check`
  - 通过

## 真实 Relay receipt

在全新临时 canonical artifact store 中，使用当前 compiler 和
`/Users/geek/workspace/internals-p5-f188` 的真实 state manifests：

1. bootstrap `skiff.run/std`；
2. publish `agine.ai/llm-api`；
3. publish `agine.ai/llm-providers`；
4. build Codex Relay package。

真实 `chatgpt_source_migration` 分析结果：

```text
migrateUnsafe:
  provenance: Analyzed [Fresh, Constant, CallerParameter(0)]
  invokesUnknownTarget: false
  requiresSameHeapIdentity: false
  returnsCallerAlias: true
  maySuspend: true

migrate:
  provenance: Analyzed [Fresh, Constant, CallerParameter(0)]
  invokesUnknownTarget: false
  requiresSameHeapIdentity: false
  returnsCallerAlias: true
  maySuspend: true
```

`returnsCallerAlias` 是 `migrateUnsafe(source)` 确实可能原样返回 formal source 的精确事实，
不是 unknown 降级；调用方传入 fresh DB value 时仍由 formal-index-aware transfer 正确消解。

Relay package artifact 已生成：

```text
skiff-package-build-v4:sha256:041341c5845d19e14c911736d6cf55c3479cf917d9cd0b2c0f05a0fc804b003e
```

deployment 继续被下一独立 `upstream_sources.adminState` 的 `invokesUnknownTarget` 阻断；该 callable
已是 `returnsCallerAlias: false`，不再归因于 migrate catch 链。本任务没有扩大修改到该 blocker。

## 不变量

- 未删除或改写 Relay catch。
- 未把 consumer 类型泛化为 unknown。
- 未把 unknown callee 标记为安全。
- 未修改 Runtime、Router、stable instance 或 consumer repository。
