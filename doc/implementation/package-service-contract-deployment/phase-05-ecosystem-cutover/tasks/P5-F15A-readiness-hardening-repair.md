# P5-F15A：Readiness Hardening Repair

## 输入、owner与限制

- 输入：D17完成；从F15原base `b91f062674df610bae2748db77a180f6279cee78`重建，不cherry-pick失败candidate
  `b7e0f4f`。
- 独立worktree/branch，一个clean commit，不merge/push。实现checkpoint合流后由integration owner唯一运行昂贵combined
  package-service test，再交R15窄复验。
- owner限`test-runner/src/runtime_execution.rs`与`runtime_execution/**`聚焦HTTP/readiness/wire模块及tests。
- 不改lib public surface、Router、Runtime、wire/receipt、fixture/source suite、manifest/Cargo.lock或stable。

## 完成态

- activation HTTP transport返回strict response与connected peer SocketAddr；health使用该peer直连、原authority Host，
  barrier内不得调用DNS或创建不可取消resolver thread；
- connect/read/write/response-size/backoff使用同一monotonic absolute deadline，单次I/O不能越过预算；
- strict `String::from_utf8`，响应schema/unknown/types fail closed；pending构造公开activation state并调用canonical
  `validate()`，不复制generation/token/participants规则；
- readiness仍要求pending null、exact active tuple、healthy+connected exact replica与同identity connected capability；
- 业务request exactly once，无503/timeout/transport retry、fallback或fixed sleep。

职责拆分为orchestration、raw HTTP transport、poll/classification与wire decode；production文件>500行或任意文件>1000行
均为阻断，除非是单一mutation corpus且有明确证据。不得复制HTTP parser或activation invariant。

## 验证

```bash
cargo test --locked -p skiff-test-runner runtime_execution
cargo clippy --locked -p skiff-test-runner --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

tests覆盖hostname+explicit peer/Host、不调用DNS、connect/read/write deadline、size cap/invalid UTF-8，pending所有canonical
mutation，完整readiness矩阵及success/503/timeout/transport业务调用均exactly once。测试server thread有界并join。
candidate完成后root可先合流为implementation checkpoint并唯一运行：

```bash
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment
```

回报commit/tree/lock、模块行数/职责、state/transport矩阵、single clean、reverse与extra-review；不宣称F04 verdict。
