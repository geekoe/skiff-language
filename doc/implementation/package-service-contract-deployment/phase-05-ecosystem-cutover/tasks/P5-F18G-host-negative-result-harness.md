# P5-F18G：Host Negative Result Harness

权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§6.2、§10、§14；F04A/R13/R14与D20
result。从D20 docs checkpoint建立`/Users/geek/workspace/skiff-p5-f18g-host-negative-harness`、
`codex/p5-f18g-host-negative-harness`。全新Agent、一个test-infrastructure commit，不merge/push/stable/full Host；
五分钟内修改。

exclusive write set仅新文件：`scripts/run-package-service-host-negative-probe.mjs`、
`scripts/lib/package-service-host-negative-probe.mjs`、`scripts/tests/package-service-host-negative-probe.test.mjs`。
禁止修改checked-in fixture、source suite、test-runner、Router/Runtime、isolated owner、I16/G16、manifest/lock。

完成态：harness复用`runInIsolatedTestRuntime`和canonical Host preparer，不跑std/`run-skiff-tests.mjs`；在task temp复制
consumer，只将副本test改为确定false assertion，checked-in fixture hash不变。task-owned透明counting proxy原样转发
Host/method/path/body到真实Router，只统计business ingress。执行canonical runner一次并记录真实non-2xx、Runtime assertion
diagnostic、runner exit1、exact一个FAIL/`1 test failed`、request count恰1且grace window无重试、PID/port/temp cleanup。
预期负例使harness自身PASS；JSON明确`fullProbeRuns:0`。

开发Agent只运行command-double，不运行真实negative：

```bash
node --test scripts/tests/package-service-host-negative-probe.test.mjs
node --check scripts/lib/package-service-host-negative-probe.mjs
node --check scripts/run-package-service-host-negative-probe.mjs
git diff --check
```

回报commit/tree/lock、command set、proxy不合成响应、fixture hash与extra-review。真实执行唯一owner是I16后全新H18 Agent。
