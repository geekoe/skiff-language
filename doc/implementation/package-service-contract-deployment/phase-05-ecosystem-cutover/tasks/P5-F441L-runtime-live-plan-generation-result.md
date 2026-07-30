# P5-F441L Runtime-live plan generation result

状态：`PASS / RUNTIME_LIVE_PLAN_GENERATION_CANONICAL`。

本 leaf 只修改 runtime-live executable plan 与 direct plan tests；没有运行 live workload，也没有修改
registry schema、test-runner、activation wire、source fixture 或 Runtime/Router production。

## 1. 输入、提交与写集

- 任务实现基线：`83543c1cd21bbb454750cbf5ee6e1d51ada987f0`。
- leaf dispatch HEAD：`0543aa9a6c29fc599565134eb2e78e0c53de614b`
  （tree `f23ff5b150eb4b6c9dbee3082aad61c410f65541`）。
- implementation：`da7a5f729c44e408a810d9d15263b13fdf026cfc`
  （tree `2b7d5cd3402b9a4727662cbd4912906022ded4ee`）。

Implementation 只修改任务允许的五个文件：

- `scripts/lib/verify-live-plan.mjs`；
- `scripts/tests/verify.test.mjs`；
- `scripts/tests/verify-live-registry.test.mjs`；
- `scripts/tests/verify-live-plan-platform-source.test.mjs`；
- 新增 `scripts/tests/verify-live-plan-runtime-generation.test.mjs`。

本文由独立 result-only commit 交付；其 commit/tree 由最终交付消息记录。

## 2. Test-first RED

先只新增四 fixture generation 断言，再运行：

```bash
node --test scripts/tests/verify-live-plan-runtime-generation.test.mjs
```

旧实现按预期得到 `1 test / 0 passed / 1 failed`，精确差异为：

```text
actual:   ['9', '9', '9', '9']
expected: ['9', '10', '11', '12']
```

继续补齐大整数与溢出用例后、修改 production 前，同一文件为
`5 tests / 2 passed / 3 failed`：重复 generation、大整数重复 generation 与越界序列未拒绝分别保持
RED。

## 3. 终态实现

### 3.1 Canonical root 与参数

- discovery 仍消费 registry 冻结的 `runtime-live-tests` handler及其确定性排序，没有复制 registry
  declaration；
- 每个 fixture 必须由精确的 `runtime/live-tests` package root 拥有；该 root 必须同时拥有
  `package.yml` 与固定 `config.skiff-test.yml`；
- caller-owned target environment 只原样传给 runner，不用于查找或验证
  `config.<environment>.yml`；
- actual repository plan 精确生成四个 phase，顺序为 DB、file、HTTP adapter、operation；
- 每个 phase 恰好携带一个 absolute `--platform-source-root`、artifact root、activation URL、ingress
  URL、target environment、expected generation、`--deny-skips` 与 `--require-tests`；
- production plan 中没有 `--base-assembly`。

F441I 后依赖 legacy root 形状的三个 stale assertion 已改为 actual canonical root 正向断言；CLI
`--list` 同样证明四个 current fixture 可生成 plan。

### 3.2 Exact generation sequence

`runtimeExpectedGenerations` 先验证 canonical unsigned-decimal syntax，再用 `BigInt` 计算完整序列并最终
转回十进制字符串。它复用 current activation owner 的
`maxExpectedAssemblyGeneration = 9007199254740990`，因此：

- 单 fixture 原样保留 caller 的 `N`；
- 四 fixture 得到唯一的 `N`、`N+1`、`N+2`、`N+3`；
- `9007199254740987` 可精确生成到 `9007199254740990`，没有经过 `Number` 加法；
- 若序列最终值超过 canonical maximum，plan 在返回任何 executable phase 前整体失败；
- 负数、正号、leading zero、小数、指数与前后空白均 fail closed。

### 3.3 Execution preflight

四个 phase 继续共享同一 execution preflight。执行前重新检查：

- 每个已发现 fixture 仍为文件；
- canonical root 的 `package.yml` 与固定 `config.skiff-test.yml` 仍存在；
- artifact root 仍为目录；
- activation/ingress URL 仍满足精确 path 契约；
- required executables 仍可用。

direct TOCTOU test 在 plan 构造后删除 fixture、package owner与固定 profile，并把 artifact directory
替换为普通文件；preflight 聚合拒绝全部变化，前后 marker command 均未执行。

## 4. 证据矩阵

| 任务条款 | 代码 / direct test 证据 | 结果 |
| --- | --- | --- |
| 单 fixture 保留 `N` | generation 专用测试的 single-fixture case | PASS |
| 四 fixture 连续唯一 `N..N+3` | generation 专用测试；actual root plan test | PASS |
| 大整数无 JS 精度损失 | `BigInt` sequence；`9007199254740987..990` golden | PASS |
| 非法 / 越界 fail closed | syntax matrix；sequence overflow matrix | PASS |
| 每 phase 一个 absolute platform root | platform-source test；四-phase policy test | PASS |
| 无 base、保留 deny/require | 四-phase policy test；actual CLI list反向断言 | PASS |
| actual canonical root positive | actual root精确四 phase IDs；registry组合 plan `4 + 1` | PASS |
| 缺 owner/profile/artifact、非法 URL/target/generation拒绝 | `verify.test.mjs` 与 registry structural matrix | PASS |
| target environment不选择profile | 只有 `config.skiff-test.yml` 时 `remote.prod` target仍成功 | PASS |
| TOCTOU root/fixture消失拒绝 | shared execution-preflight marker test | PASS |

## 5. Non-live 验证

实际执行任务规定的四文件命令：

```bash
node --test \
  scripts/tests/verify.test.mjs \
  scripts/tests/verify-live-registry.test.mjs \
  scripts/tests/verify-live-plan-platform-source.test.mjs \
  scripts/tests/verify-live-plan-runtime-generation.test.mjs
```

结果：`63 passed / 0 failed`，其中新增 generation 专用文件为 `5 passed / 0 failed`。

| 命令 | 结果 |
| --- | --- |
| 上述四文件 `node --test` | PASS，63 passed / 0 failed |
| `node --check scripts/lib/verify-live-plan.mjs` | PASS |
| `git diff --check` | PASS |

补充反向检查：

- actual `runtime/live-tests` 下 `.live.test.skiff` 精确为四个冻结路径；
- `rg -n -- '--base-assembly' scripts/lib/verify-live-plan.mjs` 为 0 命中（status 1）；
- plan production 的 generation 运算只有 `BigInt`，没有 `Number` / `parseInt` / `parseFloat`；
- 更新后的 direct tests 不再含 `legacy runtime-live` / `actual legacy runtime` 断言。

## 6. 隔离与停止条件

- 未运行 runtime-live selector workload、instance、watch、stable、Mongo、Router、Runtime、telemetry、
  固定端口或网络请求；
- CLI 覆盖只使用 `--list` 或在 plan/preflight 阶段终止；
- 未修改 registry schema、test-runner、activation wire 或任何 live source；
- 未派 sub-agent，未 merge、rebase 或 push；
- 没有发现需要越界修改的 generation owner，因此未触发 `TASK_SCOPE_EXPANDED`。
