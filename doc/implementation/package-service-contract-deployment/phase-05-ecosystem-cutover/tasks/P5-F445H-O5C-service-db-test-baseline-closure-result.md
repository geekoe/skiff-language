# P5-F445H-O5C Service-DB hermetic test baseline closure result

状态：`IMPLEMENTATION_COMPLETE / HERMETIC_FULL_GATE_GREEN`。

O5R2 记录的两个 required-full-gate 基线问题已经在测试层闭合。provider fixture 继续使用真实
publication-style `service_id`，但改用独立、确定且 Mongo-safe 的
`state_namespace = "provider_fixture"`；真实 Mongo roundtrip 保留原测试逻辑，并通过 Rust
`#[ignore = "..."]` 明确归类为需要本地 MongoDB replica set 与真实网络资源的 live 测试。

## 1. 输入与提交

| 项 | 值 |
| --- | --- |
| 直接父节点 | `P5-F445H-O5R2-service-db-prepared-runtime-operation-result.md` |
| production prerequisite | `69ba325a` |
| task document / worktree base | `fdb2e6d9` |
| test implementation | `fa93fc15` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline` |
| branch | `codex/p5-f445h-o5c-test-baseline` |

test implementation 的唯一文件是 `runtime/service-db/src/tests.rs`，且只有两处合同允许的最小
修改：

1. `provider_input` 的 `state_namespace` 从第二个 publication-style service id 改为确定的
   `provider_fixture`；
2. 为 `service_db_runtime_create_and_find_runtime_roundtrips_local_interface` 增加
   `#[ignore = "requires a local MongoDB replica set and real network resources"]`。

没有修改 production、其它 fixture、test harness、Cargo manifest、lockfile、feature、provider
namespace 规则、Mongo 校验或 recoverable 行为。

## 2. Test-first RED

修改前以离线 Cargo 和精确 selector 运行：

```text
CARGO_NET_OFFLINE=true \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline/build/cargo-target \
  cargo test -p skiff-runtime-service-db \
    mongo_provider_builds_db_capability_source_from_valid_opaque_config -- --nocapture
```

得到合同预期的单一 RED：`0 passed / 1 failed / 0 ignored / 112 filtered`。失败发生在 provider
build：

```text
valid provider config should build DB capability source:
Opaque(Decode("service id `example.com/provider_<uuid>` projects to a character forbidden in Mongo database names"))
```

测试在构造 provider source 时即失败，没有执行真实 Mongo roundtrip，也没有建立 Mongo 连接。

## 3. Green 与 full gate

所有 Cargo 命令均使用任务指定的独立 target，并额外设置 `CARGO_NET_OFFLINE=true`。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-service-db mongo_provider_builds_db_capability_source_from_valid_opaque_config -- --nocapture` | PASS：`1/1`，`112` filtered |
| `cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture` | PASS：`11/11`，`102` filtered |
| `cargo test -p skiff-runtime-service-db --locked --no-fail-fast` | PASS：unit `112 passed / 0 failed / 1 ignored`；doc-tests `0` |
| `cargo check -p skiff-runtime-service-db --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

完整 suite 共发现 `113` 项 unit test。除新归类的真实 Mongo roundtrip 外，`112` 项全部实际执行并
通过；其中
`mongo_provider_builds_db_capability_source_from_valid_opaque_config` 与
`mongo_provider_rejects_invalid_opaque_config` 均显示为 `ok`，没有被 filter 或 ignore。
O5R2 `prepared_runtime` 的 `11` 项也全部实际执行并通过。没有其它测试改变选择、计数或语义。

## 4. Hermetic 与 live 边界

普通 full gate 的 harness 输出精确显示：

```text
test tests::service_db_runtime_create_and_find_runtime_roundtrips_local_interface ...
ignored, requires a local MongoDB replica set and real network resources
```

因此测试函数体没有被 poll。反向搜索确认 `tests.rs` 中真正调用 Mongo database
`drop`/写读操作的两处都位于这个被 ignore 的同一测试体内；其余非ignored Mongo URL 测试只做
同步 metadata 校验、option 解析、lazy client handle/cache 校验或使用 test provider，不执行
Mongo database operation。Cargo 同时处于 offline 模式。普通 full gate 没有建立 Mongo
连接或访问网络，也没有读取本机 secret。

该 live 测试可由有权限的 owner 使用以下完整测试名显式运行：

```text
CARGO_NET_OFFLINE=true \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline/build/cargo-target \
  cargo test -p skiff-runtime-service-db --locked \
    tests::service_db_runtime_create_and_find_runtime_roundtrips_local_interface \
    -- --ignored --exact --nocapture
```

本节点没有执行该命令，也没有启动 MongoDB、stable、live 或 instance。

## 5. 写集与 production 证明

`fa93fc15` 相对 task base 的 diff 只有 `runtime/service-db/src/tests.rs` 的 `2 insertions /
1 deletion`。排除该测试文件后的 repository diff 为空，production diff 为零。

本节点没有 merge、rebase 或 push。
