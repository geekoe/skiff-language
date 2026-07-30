# P5-F199：Bytes.fromBase64 Callable Semantics 结果

状态：Implementation complete；真实 llm-providers/Relay receipt 等待 F190 manifest 收口

## 直接父任务

- `P5-F199-bytes-from-base64-callable-semantics.md`

## 实现结果

- 在唯一精确 native callable semantics registry 中登记
  `core.bytes.fromBase64`。
- compiler facts 与既有 native signature 和 Runtime handler 对齐：
  - 读取一个 `string`；
  - 返回每次重新分配的 fresh `bytes`；
  - 不修改 caller-reachable value；
  - 不返回或抛出 caller alias；
  - 不逃逸 caller value；
  - 不要求 same-heap identity；
  - 不调用未知目标；
  - 不挂起。
- semantics 只属于 canonical binding。`std.bytes.fromBase64`、`bytes.fromBase64` 字面别名、
  `core.bytes.fromHex` 和相似前缀均不能直接命中 registry；source resolver 必须先把合法公开
  名称解析到 canonical binding。
- Runtime 的非法 Base64 路径保持
  `RuntimeError::BytesDecode { target: "bytes.fromBase64" }`，wire code 仍为
  `std.bytes.DecodeError`。

## 正负验证

- artifact-model 验证 exact signature 为 `string -> bytes`、canonical target/aliases 以及
  全部 callable facts。
- source callable analysis 验证 `jwtPayload(value) -> bytes.fromBase64(value)` 不再产生
  unknown，return provenance 为 fresh，throw/escape provenance 为空。
- File IR 验证公开 `bytes.fromBase64` 精确 lower 为
  `core.bytes.fromBase64`；缺参、多参、错误输入类型和错误返回类型均失败关闭。
- Runtime 验证相同输入的两次调用产生不同 heap handle、内容一致，非法 Base64 继续产生
  typed bytes decode error。

## 真实链验证状态

使用本任务 Skiff worktree 和全新临时 canonical store：

- canonical `skiff.run/std@1.0.0` bootstrap 成功；
- 真实 `agine.ai/llm-api@0.1.0` publish 成功；
- 真实 `agine.ai/llm-providers@0.1.0` 在进入 codec callable acceptance 前，被直接父链已经
  单列的 F190 manifest blocker 拒绝：

  `package agine.ai/llm-providers uses database schema but declares no database state requirement`

因此本任务没有修改 llm-providers manifest、复制源码或绕过 database state gate 来伪造 receipt。
F190 合入 internals integration 后，应在同一 Skiff integration HEAD 重跑
`llm-api -> llm-providers -> codex-relay/service` publish，验收
`codec.jwtPayload -> claimsFromJwt -> importCredential` 不再带
`UnknownCallTarget`。临时 store 未读取或修改 stable store。

## 验证命令

- `cargo test -p skiff-artifact-model bytes_from_base64`
- `cargo test -p skiff-artifact-model native_callable_semantics_registry_is_sparse_exact_and_safe`
- `cargo test -p skiff-compiler-source bytes_from_base64_wrapper_uses_exact_native_semantics`
- `cargo test -p skiff-compiler bytes_from_base64_lowers_to_exact_native_binding`
- `cargo test -p skiff-runtime-native from_base64_returns_fresh_bytes_and_preserves_typed_decode_error`
- `cargo check --workspace`
- `git diff --check`

未操作 stable instance，未 push。
