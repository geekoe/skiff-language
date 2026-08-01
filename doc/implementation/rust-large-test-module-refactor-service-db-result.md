# Service DB 大型测试模块重构：开发结果

日期：2026-08-01

状态：PASS，待 integration checkpoint

任务合同：[`rust-large-test-module-refactor-service-db-leaf.md`](./rust-large-test-module-refactor-service-db-leaf.md)；
直接父节点与权威设计由该合同继续追溯。

## 代码身份与交付

- baseline：commit `805426f2249ca24d7c3b46439ac5a60be2ca3ae2`，tree
  `5db1c89a0c47e3ccf84cb564610b17f68a916c0e`。
- worktree / branch：`/Users/geek/workspace/skiff-rust-test-service-db` /
  `codex/rust-test-service-db`。
- implementation commit：`8e467ddf98941739edd6e786bdcd447735766326`，tree
  `c6168273a52a6c58038d39f8802806bf3bf4442d`。
- 集成 owner：`/root/rust_test_integrator`；本分支未 merge、push 或清理。

## 实际结构与职责

`runtime/service-db/src/tests.rs` 由 4211 行降为 13 行，终态只声明模块。102 个原根测试的唯一映射为：

| 模块 | 测试数 | owner |
| --- | ---: | --- |
| `error_contract.rs` | 11 | ServiceDbError wire/catch/constraint 与 capability-error 公共合同 |
| `provider.rs` | 3 | provider config/build/provision |
| `runtime_config.rs` | 17 | publication/storage identity、database/client cache/options |
| `metadata.rs` | 16 | metadata、collection/index/lease metadata plan 与 exact target identity |
| `mapping.rs` | 18 | key/query/document/file/Date/普通 BSON 与 RuntimeValue mapping |
| `lease.rs` | 4 | lease filter、TTL/max deadline 与 guard behavior |
| `recoverable.rs` | 16 | recoverable envelope、restore、retention 与 production-context behavior |
| `mongo.rs` | 7 | 无真实服务的 Mongo code/label/write-command conflict/retry 分类 |
| `encrypted_mapping.rs` | 9 | JSON 与 RuntimeValue encrypted mapping、伪造 metadata 拒绝 |
| `live_mongo.rs` | 1 | 唯一 ignored 本机 Mongo replica-set roundtrip |

`support.rs` 唯一拥有通用 metadata/provider/storage/Mongo-error 与跨 prepared-runtime 的 encrypted fixture；
`recoverable_support.rs` 唯一拥有 recoverable plan/heap/interface/artifact/root-store/hooks fixture。两者均无测试。
通用 `thread_binding()` 取代 18 处 Thread metadata/binding 重复，既有 target/package identity helper 收敛在
`support.rs`。错误 payload、metadata JSON 和 JSON/RuntimeValue encryption 行为输入仍保留在领域测试中。

recoverable hooks 已由 `TestDbBehaviorHooks` 的单一 `Mutex<TestDbBehaviorHookState>` 实现统一；仓库静态搜索在本测试树
只找到一个 `impl RecoverableBehaviorHooks`。四个调用计数均通过显式 accessor 读取；旧
`ThreadSafeTestDbBehaviorHooks`、`Cell`、`RefCell` 和转发 trait impl 已删除。

既有 `prepared_runtime/**` 的模块布局与 11 个测试均保持；只机械改接 `support` /
`recoverable_support`，并把使用点改为单一 hooks 类型。

当前各新文件最大为 `metadata.rs` 636 行；`support.rs` 396 行，`recoverable_support.rs` 503 行，均不拥有测试。

## 测试身份、属性与 live 边界

baseline 和实现后均执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-rust-test-service-db/build/cargo-target \
  cargo test --package skiff-runtime-service-db --lib -- --list
```

两次均退出 0、列出 crate 共 144 个测试。静态函数名/属性审计结果：

- 原根测试 102，终态十个领域模块合计 102，unique 102；missing/extra/duplicate 均为空。
- 每个函数的 `#[test]` / `#[tokio::test]` / `#[ignore = ...]` 属性数组前后完全一致，mismatch 为空。
- prepared-runtime 前后均为 11 个相同函数名，未增加、删除或改名。
- 十模块计数为 `11 + 3 + 17 + 16 + 18 + 4 + 16 + 7 + 9 + 1 = 102`。
- owner 范围内静态搜索只有一个 ignore：
  `tests::live_mongo::service_db_runtime_create_and_find_runtime_roundtrips_local_interface`；保留
  `#[tokio::test]` 与精确原因
  `requires a local MongoDB replica set and real network resources`。

`--list` 本身不证明 ignore，因此另以源属性静态审计确认。默认 suite 的运行输出把该测试报告为
`ignored, requires a local MongoDB replica set and real network resources`，未执行 live Mongo；crate 另有三个不属本
owner 的 baseline ignored 测试，因此全 crate 汇总为 4 ignored。

## 自验收矩阵

| 层级 | 命令 | 代码状态 | 结果 | 覆盖 |
| --- | --- | --- | --- | --- |
| baseline list | 上述 `cargo test ... -- --list` | baseline commit/tree | PASS，144 tests | 移动前全名/数量 |
| current list | 上述 `cargo test ... -- --list` | implementation tree | PASS，144 tests | 移动后全名/数量 |
| 静态双射 | Node 脚本解析 baseline 根文件与十个终态模块的函数名/属性 | implementation tree | PASS，102/102，0 mismatch | 函数名、test/tokio/ignore 属性 |
| prepared-runtime | Node 脚本解析 tracked `tests/prepared_runtime/**` | implementation tree | PASS，11 个函数名 | 既有 prepared-runtime 集合 |
| focused tests | `CARGO_TARGET_DIR=... cargo test --package skiff-runtime-service-db --lib --no-fail-fast` | implementation tree | PASS，140 passed / 0 failed / 4 ignored | crate lib tests；live 未执行 |
| rustfmt | `cargo fmt --manifest-path runtime/service-db/Cargo.toml -- --check` | implementation tree | PASS | 本 crate 格式 |
| Clippy | `CARGO_TARGET_DIR=... cargo clippy --package skiff-runtime-service-db --lib --tests` | implementation tree | PASS（exit 0；仅既有 advisory warnings） | crate 与 test 编译/Clippy |
| whitespace | `git diff --check` | implementation tree 前 staged diff | PASS | patch whitespace |

未运行 ignored live Mongo，未启动 router/runtime/Mongo，未运行 full verify。完整 runtime/rust-quality/checks 与联合
focused gate 仍由后续冻结候选的指定 owner 执行。

## 写集与边界审计

实际写集严格为：

- `runtime/service-db/src/tests.rs`
- `runtime/service-db/src/tests/encrypted_mapping.rs`
- `runtime/service-db/src/tests/error_contract.rs`
- `runtime/service-db/src/tests/lease.rs`
- `runtime/service-db/src/tests/live_mongo.rs`
- `runtime/service-db/src/tests/mapping.rs`
- `runtime/service-db/src/tests/metadata.rs`
- `runtime/service-db/src/tests/mongo.rs`
- `runtime/service-db/src/tests/prepared_runtime.rs`
- `runtime/service-db/src/tests/prepared_runtime/matrix/encoding.rs`
- `runtime/service-db/src/tests/provider.rs`
- `runtime/service-db/src/tests/recoverable.rs`
- `runtime/service-db/src/tests/recoverable_support.rs`
- `runtime/service-db/src/tests/runtime_config.rs`
- `runtime/service-db/src/tests/support.rs`
- 本 leaf/result 文档。

未修改 callable-effects、line gate、生产源码/API/可见性、Cargo 配置/依赖、lockfile、schema 或无关文件。
独立 target 位于 worktree ignored `build/cargo-target`；无辅助 worktree。提交 result 后再次记录 branch clean 状态，
供 integration 的 tracked Rust line-gate 集合审计使用。
