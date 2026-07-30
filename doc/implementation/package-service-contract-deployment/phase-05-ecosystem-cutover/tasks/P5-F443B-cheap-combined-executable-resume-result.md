# P5-F443B Cheap combined executable resume result

状态：`COMBINED_BLOCKED`

执行时间：2026-07-27 22:55–23:00 CST

## 结论

F443A 的两个执行遮挡都已解除：

- shared Cargo target 在约 `45 GiB` 空闲空间下从空目录成功重建，Gate A 三个 Rust target
  全部到达并通过候选断言；
- Gate B 显式注入本 gate 的 `SKIFF_ROOT` 后，Registry receipt workflow 实际运行候选
  `skiff-package-service-smoke-fixture`，`8 / 8` 通过。

但两个 Gate C canonical workflow 均到达同一个真实候选断言失败：

```text
failed to parse service source control file
/Users/geek/workspace/internals-phase-05-integration/agine/service/service.yml:
unknown field `http`, expected one of `id`, `kind`, `serviceCalls`
at line 3 column 1
```

这不是环境或命令阻塞。当前 Skiff strict reader 按权威设计正确拒绝
`service.yml.http`；精确 Internals 候选的 Agine service root 仍保留 inline
`http`、`websocket` 和 `timeout`，并且没有独立 `http.yml` / `websocket.yml`。
因此不能给出 `PASS / STABLE_CANDIDATE_READY`。

最小 owner 是 F440A 已冻结但本候选尚未闭合的 **Internals IA1 Agine one-shot external
manifest migration**，其写集为
`agine/service/{service.yml,http.yml,websocket.yml,config.dev.yml,service-api-receipt.mjs,service-api-receipt.test.mjs}`
及 `internal/agine_service_architecture.test.mjs` 中的 manifest/receipt assertions。
Skiff strict parser 不是修复 owner；给它增加 inline fallback 会违反权威设计。

## 精确候选与 provenance

| Repo | Gate 输入 HEAD | Tree | 开始 / 结束 |
| --- | --- | --- | --- |
| Skiff integration | `735bf1c46e7742d9a0589e219da5ab2a11842301` | `9d93c79ab11a5d133029e638311b322eb9619ce2` | clean / clean |
| Internals | `232094902785c6e725adafa6f4dc42137a1647b4` | `0178f3282eec1c07cdd031a365abd580fa0f204f` | clean / clean |
| skiff-packages | `19cfab5dfc827450d37e1a103d21f31f8effa4f0` | `44081bd0498919086c13adea97c07722cb768352` | clean / clean |
| F443B Skiff gate worktree | `735bf1c46e7742d9a0589e219da5ab2a11842301` | `9d93c79ab11a5d133029e638311b322eb9619ce2` | clean / result-only change |

任务冻结的 Skiff production anchor 是
`7d6f4de8f01392f33d31237df3050dddd305fdb7`，tree
`770d7fa75e8d45410eae9861e288b884d2ac9cd9`。执行前实际 dispatch HEAD
`735bf1c4` 相对该 anchor 的唯一变化是：

```text
A doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F443B-cheap-combined-executable-resume.md
```

因此生产源码相对任务冻结 anchor bit-identical；没有把调度文档提交误当成候选代码变化。
Internals 与 skiff-packages 精确匹配任务指定 commit。所有结束状态的
`git diff --check` 和 cached diff check 均为零。

## Gate A：恢复的三个 Rust target

所有命令均在 `/Users/geek/workspace/skiff-phase-05-integration` 执行，并使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-host websocket_jsonrpc --no-fail-fast` | PASS；主 unit binary `17 passed / 0 failed / 284 filtered out`；另三个 integration binaries 为 0 个匹配测试 |
| `cargo test -p skiff-runtime-package-test --test package_artifact entrypoint_validation_rejects_non_exact_gateway_facts` | PASS；`1 passed / 0 failed / 7 filtered out` |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment` | PASS；`28 passed / 0 failed / 1 ignored` |

Gate A 合计实际执行 `46` 个通过测试，`0` 失败，另有 `1` 个明确 ignored identity probe。
已有 compiler/source 与 runtime/linker warning 不改变退出状态。三条命令均未再出现 ENOSPC。

## Gate B：修正后的 Registry receipt

在 `/Users/geek/workspace/skiff-packages-phase-05-integration` 执行：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f443b-cheap-combined-resume \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  node --test \
  scripts/registry-service-source.test.mjs \
  scripts/registry-service-receipt.test.mjs
```

结果：

```text
tests 8
pass 8
fail 0
skipped 0
```

其中 F443A 因错误 root 中止的
`fresh Registry publish and assembly receipts close twenty service operations`
本次完整通过。receipt 输出：

```text
receiptOperationIds=20
serviceContractOperations=20
deploymentOperationBindings=20
gatewayEntries=0
deploymentIngress=0
assemblyGatewayIngress=0
```

并生成：

```text
PackageBuildId =
skiff-package-build-v10:sha256:9fc67ab14511fc9080fa1a30c6778188333c2bc957691db3339690d160a79e57
ServiceProtocolIdentity =
skiff-service-protocol-v5:sha256:d8825672efdce323ae716e8f78152b14ec5b915f9a1eb08637be1c9b7fbc238c
DeploymentRevision =
sha256-8179392934d965abbd05334d24d9c36d5732ec2fb51d9205bbe7f84e29307cef
AssemblyIdentity =
skiff-runtime-assembly-v2:sha256:5ee6b371c244d7e6dcdf1116fd055ff4e9a40faba03a3e231bd5be38c7324d50
```

`registry-service-receipt.test.mjs` 调用
`skiff-package-service-smoke-fixture`；显式选择的候选
`test-runner/Cargo.toml` 声明该 bin。此前 main root 缺 bin 的同一 workflow 本次成功产生
完整 receipt，证明没有回退 `/Users/geek/workspace/skiff` main toolchain。

## Gate C：两个 isolated service graph

两个命令都显式使用：

```text
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f443b-cheap-combined-resume
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| cwd / 命令 | 结果 |
| --- | --- |
| `aihub/service` / `npm run type-check` | FAIL，exit `1`；package publish 阶段读取 Agine `service.yml` 时拒绝 inline `http` |
| `agine/service` / `npm run type-check` | FAIL，exit `1`；`prepare-packages --list --fixture-only` 先成功，随后同一 package publish 断言失败 |

Agine preflight 清单记录的 provenance 正是本结果表中的三棵指定 checkout，并列出 `6` 个
package source root、`4` 个 service source root。两个 workflow 都已越过 F443A 的 shared-target
编译遮挡；失败发生在候选 source trust boundary，尚未产生最终 type-check/test count。

权威设计
`package-service-contract-deployment.md` 第 2、3 节规定 HTTP / WebSocket external ingress
分别由 `http.yml` / `websocket.yml` 拥有，旧 `service.yml.http` /
`service.yml.websocket` 必须直接报错且无兼容读取。当前候选事实为：

```text
agine/service/service.yml:3:http:
agine/service/service.yml:292:websocket:
agine/service/service.yml:297:timeout:
```

同目录没有 `http.yml` 或 `websocket.yml`。Skiff
`compiler/input/src/service_config.rs` 已分别读取 strict `service.yml`、可选 `http.yml` 和
可选 `websocket.yml`；故最早的 `http` unknown-field 错误是预期 fail-closed 检查发现未迁移
consumer，而不是 parser 回归。

## 环境、隔离与清理

- 执行前 APFS data volume 约 `45 GiB` 可用；结束时 shared target 为 `7.4 GiB`，volume
  约 `37 GiB` 可用、`91%` 使用。没有 ENOSPC。
- Node 临时根为
  `/var/folders/v2/l4swjmr50s721ntxp56n759h0000gp/T`。结束时
  `internals-canonical-assembly-*` 与 `skiff-registry-receipt-*` 均无残留。
- 本 gate 没有创建临时 `node_modules` symlink；gate worktree 的 root 与
  `router/node_modules` 均不是 symlink。
- 两个 Internals workflow 使用 `mkdtemp` 隔离 ecosystem store，失败路径已执行清理；没有触碰
  stable watch registry、stable artifact root、reload endpoint、MongoDB、固定端口或 live/network。
- 没有修改候选源码、fixture、manifest、lockfile或配置；没有修复 blocker；没有派子 Agent；
  没有 merge、rebase 或 push。

本节点只新增本 result。
