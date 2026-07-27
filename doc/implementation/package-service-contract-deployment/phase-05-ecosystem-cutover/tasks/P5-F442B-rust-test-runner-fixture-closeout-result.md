# P5-F442B Rust / Host / test-runner fixture closeout result

状态：`PASS / R_NODE_CLOSED / NON_LIVE_ONLY`。

本 leaf 已关闭 package-test、Host 与 test-runner 的 current-positive fixture 漂移，没有修改
Rust production 结构、compiler/artifact 算法、cross-system corpus、checker、README 或其它
task/result。没有发现需要 production owner 或公共语义变更的 golden，因此不返回
`TASK_SCOPE_EXPANDED`。

## 1. 基线与提交

| 项目 | 值 |
| --- | --- |
| 实现基线 | `0303fe5d` |
| task start HEAD | `2989ddf97391e97b99c3c1dd8c3d9468de0d28f7` |
| worktree | `/Users/geek/workspace/skiff-p5-f442b-rust-fixtures` |
| branch | `codex/p5-f442b-rust-fixtures` |
| implementation commit | `8d8317dc0d8ad30bc8589acaf8adbe38234f15ba` |
| result commit | 本文独立提交；最终 commit 由交付消息记录 |

## 2. 真实 RED

在任何 fixture 修改前运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-package-test --test package_artifact \
  entrypoint_validation_rejects_non_exact_gateway_facts
```

命令以 exit `101` 失败，编译器准确报告两个 `E0063`：

- `PackageRequirement` initializer 缺少 `collection_name_mapping`；
- `PackageBinding` initializer 缺少 `collection_name_mapping`。

完成基础 fixture 修复后，首次运行 test-runner integration 得到
`27 passed / 1 failed / 1 ignored`。唯一失败是
`ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures` 的旧 build golden：

```text
actual   skiff-package-build-v10:sha256:87120182...50ad
expected skiff-package-build-v10:sha256:5ce08903...25880
```

这两个失败都来自真实目标入口，不是 synthetic probe、零测试或依赖安装失败。

## 3. 实现结果

- 两个 package dependency initializer 都使用 `BTreeMap::new()` 表示语义正确的空 collection
  mapping，没有修改 production model。
- Host current-positive `RuntimeAssembly` fixture 刷新到 v2，current-positive
  `ServiceProtocol` fixture 刷新到 v5。
- 保留 `runtime/host/tests/active_runtime_assembly.rs` 中用于 unknown-resolution reject path 的
  RuntimeAssembly v1 stale negative；text runtime WebSocket 的 legacy request rejection probe
  也未改。
- 删除仅由 `#[cfg(test)]` 声明的旧 `register_mapper` module 与 470 行自包含旧 receive/Gateway
  v1 mapper 测试。反向搜索 `runtime/host/src` 与 `runtime/host/tests` 中
  `register_mapper` 为零。
- test-runner orchestration 的 current-positive `DeploymentArtifact` fixture 刷新到 v3。
- WebSocket source fixture 不是逐个追首个失败；先让四个 fixture 完整生成，再一次性采集并核对
  build/ABI/deployment/assembly 四元组，最终以单个 tuple assertion 固定每组完整值。

完整 current tuple 为：

| fixture | package build | local ABI | deployment | assembly |
| --- | --- | --- | --- | --- |
| `package-service-websocket-smoke` | `87120182...50ad` | `d5627a25...b9ba` | `5b6d9b94...cf1f` | `fe087b09...d0b5` |
| `package-service-websocket-generation-a` | `c6573cde...2f94` | `d5627a25...b9ba` | `7aef5d6e...d1bb` | `c08e8c51...ce0a` |
| `package-service-websocket-generation-b` | `b50bdc91...6772` | `d5627a25...b9ba` | `c624240f...c2c8` | `15214138...26f0` |
| `package-service-i02-spawn-submit` | `71d27387...24fe` | `3db7056f...cdf9` | `7971d36a...0796` | `938e19fe...2654` |

表中省略号只用于结果文档可读性；测试源码固定并比较完整 identity。

## 4. 规定的 non-live 验证

| 命令 | 结果 |
| --- | --- |
| package-test 指定 test | `1 passed / 0 failed / 0 ignored / 7 filtered out` |
| `cargo test -p skiff-runtime-host --lib` | `301 passed / 0 failed / 0 ignored` |
| `cargo test -p skiff-test-runner --lib` | `41 passed / 0 failed / 2 ignored` |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment` | `28 passed / 0 failed / 1 ignored` |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

四个 Rust 命令均使用任务指定的
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。
编译期间只有既有 advisory warnings，没有失败或新增 gate。

## 5. 范围与运行时审计

- implementation commit 只包含 leaf 允许的 10 个变更/删除路径；允许写集中的
  `runtime/host/tests/active_runtime_assembly.rs` 因其 v1 是 deliberate negative 而保持不变。
- 没有启动 stable instance、MongoDB、watch、server、network、live selector 或完整 workspace
  suite。
- 未派子 Agent，未 merge、rebase 或 push。
