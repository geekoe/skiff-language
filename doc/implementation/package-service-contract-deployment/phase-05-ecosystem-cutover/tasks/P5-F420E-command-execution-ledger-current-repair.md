# P5-F420E Command execution ledger current repair

状态：Ready（F420D scope-expansion 后继）。

## 直接父节点

- `P5-F420D-remove-obsolete-tar-oracle-and-final-gate-result.md`

父节点已经证明，当前 tooling gate 的唯一首错是 command-execution ledger 与三个现状不一致：

- `scripts/skiff.mjs` 已不存在 `spawnBrowserChild` 调用或 import，但 ledger 仍登记
  `browser-unref`；
- `scripts/lib/isolated-test-runtime.mjs` 有一个真实 `spawn`，其 child 被
  `additionalRuntimes` 保留，并在 cleanup 中执行 `SIGTERM`、超时 `SIGKILL` 和 exit await；
- `scripts/lib/platform-source-probe-support.mjs` 有一个真实 `spawn`，其 detached process
  group 在 abort/close 后执行 TERM/KILL 退休检查，并核对已观察端口关闭。

本节点只让 canonical ledger、调用点 owner marker/alias 和精确数量断言反映以上现状；不得改变
既有进程生命周期。

## 精确起点与 DAG

- integrated start：
  `9cf956b561e249e9dc15e44431360b748dca85a8`；
- tree：
  `0bdddf37e77a0d3fd9731387118af31bea4d1e8f`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 start/tree 与 F415 ancestry。完成后解除只读 N4 final gate；本任务自身不得宣称
F421 已解除。

风险：低。验收分组：F420 N4 tooling repair。当前候选成熟度：预验收候选。

## 唯一允许写入

```text
scripts/lib/command-execution-ledger.mjs
scripts/lib/isolated-test-runtime.mjs
scripts/lib/platform-source-probe-support.mjs
scripts/tests/command-execution-policy.test.mjs
本任务 result
```

`scripts/skiff.mjs`、`scripts/lib/command-execution-policy.mjs`、
`scripts/lib/command-execution-scanner.mjs` 仅允许读取和反向搜索，不得修改。不得修改 Router、
Rust、其它 test、验证计划、锁文件或生态仓库；不得 merge/rebase/push、访问 stable/live、
instance 或 watch registry。

任务有单一明确路径，不派子 Agent。从启动到第一次代码修改不超过五分钟；发现需要改变生命周期、
owner class 集合、scanner/policy 语义或其它 production owner 时，立即返回
`TASK_SCOPE_EXPANDED`。

## 必须实现

1. 从 ledger 删除已经不存在的 `scripts/skiff.mjs` / `browser-unref` owner。
2. 给 `isolated-test-runtime.mjs` 的真实 spawn 使用唯一、说明用途的 import alias，并在直接调用前
   添加 scanner 识别的 owner marker；ledger 登记：
   - owner function 必须是实际直接调用所在函数；
   - owner class 必须反映“保留 child handle 并由 isolated runtime cleanup 管理”的既有事实；
   - reason 必须准确描述 TERM/KILL/await cleanup，不得声称不存在的保证。
3. 给 `platform-source-probe-support.mjs` 的真实 spawn 使用唯一、说明用途的 import alias，并在直接
   调用前添加 owner marker；ledger 登记：
   - owner function 必须是实际直接调用所在函数；
   - owner class 必须反映“owned detached process group”的既有事实；
   - reason 必须准确描述 abort、close、TERM/KILL retirement 与端口核对。
4. 更新 policy test 的 current 精确数量：删除一个 stale spawn owner、增加两个真实 spawn
   owner后，应为 `12 total / 10 spawn / 2 execFile`，owner id 仍全部唯一。
5. 不得增加 whole-file exception、migration pending、scanner skip、路径例外、额外 callCount，
   也不得把真实调用改经不受 ledger 检查的间接引用。

## 聚焦验证

在本任务 worktree 执行：

```bash
node --test scripts/tests/command-execution-policy.test.mjs
node scripts/verify.mjs --only tooling

rg -n "spawnBrowserChild|browser-unref" \
  scripts/skiff.mjs scripts/lib/command-execution-ledger.mjs \
  scripts/tests/command-execution-policy.test.mjs

cargo fmt --all -- --check
git diff --check
```

预期：

- policy test 精确 `10/10`；
- tooling 全部通过，command-caller 仍为 `3/3`；
- stale browser owner 反搜为 0；
- production discovery 中所有 `node:child_process` import 均由 ledger 精确覆盖；
- 相对起点，除授权文件和 result 外零 diff。

不要运行 Router、test-runner、`run-skiff-tests` 或其它最终 N4 gate；它们由合流后冻结候选上的
独立 gate owner 唯一执行。

## 交付

实现与 `P5-F420E-command-execution-ledger-current-repair-result.md` 分开提交。result 记录：

- start、implementation 与 final commit/tree；
- 两个 owner 的 alias、marker、owner function、owner class 与既有 lifecycle 证据；
- stale owner 删除和 `12 / 10 / 2` 数量；
- policy `10/10`、完整 tooling 计数、反向搜索、格式与 diff；
- worktree clean，未 merge/rebase/push/stable/live；
- 是否可以建立 N4 冻结候选。

