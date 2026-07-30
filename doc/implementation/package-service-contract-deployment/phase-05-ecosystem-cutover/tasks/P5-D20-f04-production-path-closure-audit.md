# P5-D20：F04 Production Path Closure Audit

## 熔断与目标

同一真实入口`node scripts/run-skiff-tests.mjs`到最终Host结果已经连续暴露多个跨层blocker，正式触发跨层收敛熔断。
在本任务完成、独立repair wave合流、I16 combined PASS与R16 PASS前，禁止再次运行完整source-suite/Host。本任务不改变
权威设计，只把现有production path闭合为一次可汇总的事实矩阵。权威设计为
`doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14及阶段标准2/4/5/6。

## 并行只读分片

由三个全新、互不复用的只读Audit Agent并行执行；不得编辑、提交、运行昂贵完整probe或分别给阶段/F04 verdict。
每个Agent只返回事实、缺口和便宜probe；root是唯一aggregate owner，在三份结果及F16B/F16C exact commits可见后一次
汇总矩阵并批量更新repair DAG。

- **D20A source/compiler/artifact/fixture**：source registry/default root、isolated config/env/ports/source artifact root、
  canonical generation、compiler platform context/manifests/prelude、std project/test overlay/artifact assembly、Host fixture
  authoring/receipt/contracts/packages/deployments/base assembly。F16B/F16C在途surface先标为pending，提交后在exact diff上
  复核事实；不修改它们。
- **D20B Router/Runtime activation/readiness**：isolated bootstrap/supervisor、Router endpoint/control/capabilities、durable
  committed bootstrap、prepare/admit/commit/register、active tuple/healthy replica/readiness/generation与生命周期。
- **D20C request/eval/observable/cleanup/env**：Host ingress与canonical request、Router dispatch、Runtime strict decode/eval、
  service boundary、provider/helper same-heap mutation、response assertion，以及teardown/ports/process/worktree/target provenance。

若发现必须改变公共契约、架构职责或业务语义，受影响分支标记`DESIGN DECISION REQUIRED`并交root向用户升级；其他分片
继续。设计不变的独立blocker按production owner拆为新的有界开发节点，禁止追加给既有长期Agent。

## 闭合矩阵

每一行必须使用：

```text
jump | production owner | input | output | positive probe | negative probe | evidence exact commit | unseen/hidden | cheap probe
```

矩阵不得遗漏以下真实跳点；相邻跳点可以由同一owner实现，但不得合并掉边界：

1. source registry/default root；
2. isolated config/env/ports/source artifact root；
3. bootstrap canonical generation；
4. compiler platform context/manifests/prelude；
5. std project/test overlay/artifact assembly；
6. Host fixture authoring/receipt/contracts/packages/deployments/base assembly；
7. supervisor/Router/Runtime startup、control endpoint与capabilities；
8. committed recovery/registration/readiness；
9. activation prepare/admit/commit；
10. Host ingress canonical request；
11. Router dispatch/Runtime decoder/eval；
12. service boundary/provider/helper same-heap mutation；
13. response/result exact assertion；
14. cleanup/ports/process/worktrees/shared-target provenance。

证据应优先复用R08–R15、F04A/B、D18/F16、D19/F17的exact ledger，并明确哪些事实因候选变化失效。`unseen/hidden`
必须说明尚未检查、只由mock覆盖或曾被上游失败遮挡的范围；不能把未触发等同PASS。便宜probe只允许static search、
direct unit/integration test、command-double、schema/corpus、构建检查或不启动完整链路的局部动态检查。

## 汇总与退出

root在同一exact candidate上产出一份D20 aggregate result：包含14行闭合矩阵、三分片evidence、设计问题、独立blocker、
真实依赖/写入冲突、可并行repair nodes及每个节点的owner/输入/退出条件。只在所有未遮挡范围有事实或明确repair节点后
关闭audit。随后一波合流所有相关repair，先运行I16唯一cheap combined probe；失败直接退回repair，PASS后才启动新的
R16，最终由G16运行一次完整Host。当前收敛周期同一完整probe原则上最多两次；第三次前必须重做剩余范围审计并说明。
