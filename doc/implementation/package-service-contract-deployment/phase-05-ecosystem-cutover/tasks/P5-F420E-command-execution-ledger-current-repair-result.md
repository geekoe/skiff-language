# P5-F420E Command execution ledger current repair result

状态：`TASK_SCOPE_EXPANDED`。Command-execution ledger 已按授权反映 current production，
policy 精确 `10/10` 通过；完整 tooling 继续前进后暴露一个与本任务零 diff 的
crate-public-api gate fixture 漂移。修复需要修改未授权的其它 test，因此本节点不能建立 N4
冻结候选，也不宣称 F421 已解除。

## 1. Exact candidate 与 implementation checkpoint

- integrated start / tree：
  `9cf956b561e249e9dc15e44431360b748dca85a8` /
  `0bdddf37e77a0d3fd9731387118af31bea4d1e8f`；
- task checkout / tree：
  `1c557d25ab841a75fca1e59de19c747b61e8cff9` /
  `80effb5edf006d4171e36c4d9c6e7567c077f5cd`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- implementation checkpoint / tree：
  `b5feb24962ae460d6a2e30795e1455f93ad2b485` /
  `577ee7f66477e3b42cc374ec288c4f1c35a2e2d5`；
- final executable candidate / tree：
  `b5feb24962ae460d6a2e30795e1455f93ad2b485` /
  `577ee7f66477e3b42cc374ec288c4f1c35a2e2d5`。

启动时 integrated start 与 accepted F415 均经 `git merge-base --is-ancestor` 验证为 HEAD
ancestor，integrated start tree 精确匹配。task checkout 只在 integrated start 之上增加本任务
文档。最终 result-only commit/tree 由交付消息记录；它不改变上述已验证的 executable candidate。

## 2. Current owner 收敛

### 2.1 Isolated additional Runtime

| 字段 | current 值 |
| --- | --- |
| import alias | `spawnAdditionalRuntimeChild` |
| owner marker / id | `child-process-owner: isolated-additional-runtime` |
| owner function | `startIsolatedTestRuntime` |
| owner class | `managed-component` |

调用返回的 child 立即进入 `additionalRuntimes`，并同时进入成功 stack 与启动失败 partial stack；
`cleanupIsolatedTestRuntime` 遍历这些 retained handles 调用 `stopAdditionalRuntime`。后者先发
`SIGTERM` 并等待 exit，20 秒超时后发 `SIGKILL`，最后继续 await 同一个 exit promise。ledger
reason 只描述这些既有事实；本任务没有改变 lifecycle。

### 2.2 Platform source probe process group

| 字段 | current 值 |
| --- | --- |
| import alias | `spawnPlatformSourceProbeChild` |
| owner marker / id | `child-process-owner: platform-source-probe-group` |
| owner function | `captureOwnedCommand` |
| owner class | `owned-process-group` |

current 调用在非 Windows 上以 `detached: true` 建立 process group。abort handler 向 child/group
发送 `SIGTERM` 并安排超时 `SIGKILL`；await `close` 后，`retireProcessGroup` 再以
TERM/KILL 和存活轮询确认退休，随后对观察到的 lease ports 调用 `assertPortsClosed`。ledger
reason 精确覆盖 abort、post-close retirement 与端口核对；本任务没有改变 lifecycle。

## 3. Ledger 与 policy 数量

- 删除已经不存在的 `scripts/skiff.mjs` / `spawnBrowserChild` / `browser-unref` ledger owner；
- 增加上述两个真实 spawn owner；
- ledger 精确为 `12 total / 10 spawn / 2 execFile`；
- 12 个 owner id 全部唯一；
- 没有新增 owner class、whole-file exception、migration pending、scanner skip、路径例外或额外
  `callCount`；
- `scripts/lib/command-execution-policy.mjs` 与
  `scripts/lib/command-execution-scanner.mjs` 均为零 diff，真实调用仍是 scanner 可见的直接调用。

精确反向搜索：

```bash
rg -n "spawnBrowserChild|browser-unref" \
  scripts/skiff.mjs scripts/lib/command-execution-ledger.mjs \
  scripts/tests/command-execution-policy.test.mjs
```

结果为 0。policy 的 actual-production test 同时证明 production discovery 中每个
`node:child_process` import 都由 ledger 精确覆盖。

## 4. 验证结果与新 blocker

| gate | 结果 |
| --- | --- |
| `node --test scripts/tests/command-execution-policy.test.mjs` | 10/10 PASS |
| artifact identity validation | 7/7 PASS |
| identity single-source self-test | 1/1 PASS |
| command-caller migrations | 3/3 PASS |
| command-execution policy（tooling 内） | 10/10 PASS |
| command execution | 13/13 PASS |
| compiler boundaries | 10/10 PASS |
| crate public API characterization | 82/82 PASS |
| crate public API CLI | 4/4 PASS |
| crate public API gate | 4/5 PASS；新首错 |
| tooling 到停止点合计 | 134 passed / 1 failed |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |
| stale browser owner 反搜 | 0 |

`node scripts/verify.mjs --only tooling` 的新首错为：

```text
configured public API crate(s) missing from workspace: skiff-deployment
```

`scripts/tests/crate-public-api-gate.test.mjs` 用
`MANAGED_CRATE_NAMES.slice(1)` 构造缺项，所以 current 缺失项必然是
`MANAGED_CRATE_NAMES[0]`，即 `skiff-deployment`；但同一 test 仍硬编码要求错误文本匹配
`compiler-contract`。该 test 与相关 crate-public-api production files 相对 integrated start 和
本 implementation 均为零 diff。最小后继需要把这个 fixture 断言收敛到 current 首个 managed
crate（或从 policy 值派生预期），但该文件不在本任务唯一允许写入内，故立即停止且不做范围外修复。

## 5. 边界与 N4 判断

implementation 只修改四个授权文件；result 是唯一新增文档。没有修改 Router、Rust、其它 test、
验证计划、锁文件或生态仓库，没有运行 Router、test-runner、`run-skiff-tests` 或其它最终 N4
gate。没有 merge、rebase、push，也没有访问 stable/live、instance 或 watch registry。

Command-execution repair 本身已闭合，但完整 tooling 未在同一 exact candidate 上通过，因此当前
**不能建立 N4 冻结候选**，F421 **未解除**。
