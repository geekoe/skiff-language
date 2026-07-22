# P5-H18：Host Focused-negative Execution

使用未参与F18G、D20、I16或其它验收的全新执行Agent。权威设计：架构§3、§6.2、§10、§14。输入为同一final
candidate/lock、F18G command-double ledger与I16 PASS bundle；前后clean，且本candidate无H18执行记录。第一行只给事实
`H18 PASS`或`H18 FAIL`，不作F04/R16/阶段verdict、不修复。

不得编辑/提交/操作stable、运行command-double/I16/std/`run-skiff-tests.mjs`/完整Host。唯一真实命令无参数：

```bash
cd /tmp
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-package-service-host-negative-probe.mjs
```

一旦启动即消耗唯一H18 attempt；失败、输出丢失或环境异常均不重试。PASS要求末行JSON同时满足：schema v1、
`verdict:PASS`、`fullProbeRuns:0`、`negativeProbeRuns:1`、preparer/runner各1、sourceSuite 0、checked-in fixture SHA不变、
runner exit1、exact一个FAIL/1 failed test、真实4xx/5xx含Runtime `assertion failed`、proxy request1/retry0/
synthetic0且500ms grace无第二次、PID/ports/proxy/temp cleanup均true。

H18不计full-probe budget，不能替代G16。candidate、fixture/harness、preparer/runner、isolated boundary、Router/Runtime
request/result surface或I16 bundle变化即失效。
