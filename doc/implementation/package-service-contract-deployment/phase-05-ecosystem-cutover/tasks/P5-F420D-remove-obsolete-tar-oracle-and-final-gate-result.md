# P5-F420D Remove obsolete tar oracle and final N4 gate result

状态：`TASK_SCOPE_EXPANDED`。失去 production owner 的 tar oracle 已按授权完整删除，direct
command-caller tests精确收敛为 3/3，tar production/test反搜为0；但完整 tooling gate继续前进后
暴露 command-execution production ledger与当前调用点不一致。修复需要修改本任务未授权的
`scripts/lib/**`、`scripts/skiff.mjs` 及其 policy test，因此 N4未判为PASS，F421 **未解除**。

## 1. Exact candidate 与 implementation checkpoint

- integrated start / tree：
  `9ed8c2bcd2918d4f1b60ab5c5ceeefa6519d68eb` /
  `192f892b6744a8515443fb52303053398495c1b7`；
- task checkout / tree：
  `0b36c4d5fa9930f3ae9c027f649ab8666836ce7a` /
  `809bd323460ee4281d2c213ee9729c3c6c0dd4b5`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- implementation checkpoint / tree：
  `3b072b4e4b52248d544ac2736d6ec9d1e800adcf` /
  `1290ee7a619bd02f6fb3383371252b067617164d`。

启动时 integrated start 与 accepted F415均经 `git merge-base --is-ancestor` 验证为 HEAD
ancestor；integrated start tree精确匹配。task checkout只在 integrated start之上增加本任务文档。

## 2. Obsolete tar oracle 删除

从 `scripts/tests/command-caller-migrations.test.mjs` 删除：

- 整个 `missing tar is reported through the safe outcome failure before remote I/O` test；
- 随之 unused 的 `skiffCli` constant。

共删除30行，没有改动剩余三个测试语义。direct execution同时提供精确 discovery count：

```text
tests 3
pass  3
fail  0
```

保留并实际通过：

1. instance status missing lsof / missing ps fallback；
2. 真实 workspace ownership/config receipt下的 child exit 9与invalid JSON；
3. runtime/compiler DAG adapters的current missing-cargo safe outcome。

全仓 production与test反搜：

```bash
rg -n \
  "failed to spawn tar|Command::new\\(\"tar\"|captureAttachedCommand\\(['\"]tar|spawn\\(['\"]tar" \
  --glob '*.{rs,mjs,ts}'
```

结果为0。没有复制成重复的 missing-cargo test，也没有修改 production。

## 3. 完整 tooling gate 的新 blocker

精确命令：

```bash
node scripts/verify.mjs --only tooling
```

执行到新首错前的实际计数：

| phase | 结果 |
| --- | --- |
| artifact identity validation | 7/7 PASS |
| identity single-source self-test | 1/1 PASS |
| command-caller migrations | 3/3 PASS |
| command-execution policy | 9/10 PASS |

policy首项失败的精确 violations：

```text
scripts/lib/isolated-test-runtime.mjs:1
  unregistered child_process import spawn as spawn

scripts/lib/platform-source-probe-support.mjs:2
  unregistered child_process import spawn as spawn

scripts/skiff.mjs ledger owner browser-unref expected 1 direct call(s)
  through spawnBrowserChild, found 0

scripts/skiff.mjs ledger owner browser-unref expected exactly one import
  spawn as spawnBrowserChild, found 0
```

当前 ledger在 `scripts/lib/command-execution-ledger.mjs` 冻结为11项：

- `scripts/skiff.mjs` 的 `browser-unref` owner已经 stale；
- `scripts/lib/isolated-test-runtime.mjs` 的实际 spawn owner未登记；
- `scripts/lib/platform-source-probe-support.mjs` 的实际 spawn owner未登记；
- `scripts/tests/command-execution-policy.test.mjs` 仍断言 exactly eleven owners，以及
  `9 spawn / 2 execFile`。

以下 production/policy owner相对 integrated start均为零diff：

```text
scripts/lib/isolated-test-runtime.mjs
scripts/lib/platform-source-probe-support.mjs
scripts/skiff.mjs
scripts/lib/command-execution-ledger.mjs
scripts/lib/command-execution-policy.mjs
scripts/tests/command-execution-policy.test.mjs
```

最小后继需要独立授权审计两个现存 spawn的生命周期 owner/marker与owner class，删除 stale browser
ledger项、登记两个真实owner，并同步policy test的精确数量。不能只放宽 scanner或跳过实际
production discovery。该修改横跨 production ledger与另一个test，超出F420D唯一允许写入的
command-caller test。

## 4. 已执行与未执行门禁

| gate | 结果 |
| --- | --- |
| direct command-caller listing/execution | 3 listed；3/3 PASS |
| tar command/oracle反搜 | 0 |
| `node scripts/verify.mjs --only tooling` | FAIL；20 passed / 1 failed before stop |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

按照任务的停止条件，新范围外失败出现后停止，没有继续运行或伪报：

- Node五文件组与两个 identity checker；
- `node scripts/verify.mjs --only router`；
- test-runner listing/execution；
- `node scripts/run-skiff-tests.mjs`。

F420C父节点已经保留 Router 608/608等证据；本候选相对 integrated start没有修改
`router/**`、F415 mapping、production或其它test。但最终N4要求本任务列出的全部命令在同一
exact tree通过，不能用继承证据替代本次完整tooling gate，因此N4仍为FAIL，F421未解除。
