# P5-F196：DB Write Callable Effect/Provenance Transfer 结果

状态：Completed

## 直接父任务

- `P5-F196-db-write-callable-transfer.md`

## 实现

- `db transaction value` 不再无条件产生 `unsupportedControlFlow` 和 unknown：
  - analyzer 按顺序转移前置语句；
  - 最终表达式的真实 provenance 原样成为 transaction 结果；
  - transaction 仍精确标记 `maySuspend`。
- DB insert/update/upsert/change body 中的静态字段投影按真实持久化编码边界处理：
  - `row.field`、`input.nested.field` 写入 BSON 后不再保留源 heap identity；
  - 直接写入 caller-owned record 仍是 Database escape；
  - 动态调用、未知 predicate 和未知 update value 仍按 unknown fail closed。
- 没有把 transaction 返回值伪造为 Fresh。Runtime 的 value transaction 原样返回最终
  `RuntimeValue`，因此 fresh receipt 中嵌入 caller-owned nested record 时，
  `returnsCallerAlias` 仍保留，交由真正执行 canonical value encoding 的 boundary owner消费。

## 正负探针

- DB read、insert、update、upsert 的已分离 scalar/record field 写入；
- DB value transaction 的 fresh outer record + nested caller alias；
- DB value transaction 直接返回 caller value；
- 直接持久化 caller-owned mutable record；
- unknown predicate 与 unknown update call。

所有 unknown、direct alias、same-heap 负例均未放宽。

## 真实验证

在临时 canonical artifact store 中完成：

1. bootstrap `skiff.run/std@1.0.0`；
2. publish `agine.ai/llm-api@0.1.0`；
3. build 带真实 database state requirement 的
   `agine.ai/llm-providers@0.1.0`。

F195 与本任务合并后的真实 llm-providers artifact：

```text
packageBuildId:
skiff-package-build-v4:sha256:c394745c1f29832c9c4da7a86a881931d2ba973cfb3f5729d6e53f1997190ec4
```

该构建证明 DB write/source transfer 已越过原断面，同时暴露了一个独立、较早的 native
callable blocker：

```text
chatgptPlan.importCredential
→ codec.tokenClaims
→ codec.claimsFromJwt
→ codec.jwtPayload
→ bytes.fromBase64
→ core.bytes.fromBase64 缺少 exact callable semantics
```

真实逐层诊断确认 `safeSecret`、`safeAccountMetadata`、`trimmed` 已是 exact；
`jwtPayload` 才首次成为 `unknownCallTarget`。因此本任务没有把该 unrelated native call
伪装为 DB fresh/known；Relay 最终 receipt 必须在补齐该精确 native semantics 后重跑。

同一 DB transfer 提交由 F194 在真实 Registry canonical build 中复验：

```text
Available: 20
Package-only: 0
```

四个 Put 与四个 PointerCas 均已越过原先的 transaction
`unsupportedControlFlow`/unknown 断面；CAS 的真实 nested return alias 由 F194 的
canonical boundary value encoding 条件消费，不由 source analyzer 伪造 Fresh。

## 测试

- `cargo test -p skiff-compiler-source --lib callable_effects --no-fail-fast`
  - 38/38 通过。
- `cargo check --workspace`
  - 通过。
- `git diff --check`
  - 通过。
- 真实 llm-api publish、llm-providers build
  - 通过。

全仓 `cargo fmt --all -- --check` 会命中 integration 基线中与本任务无关的既有格式差异；
本任务三个 Rust owner 文件已独立通过 `rustfmt`。
