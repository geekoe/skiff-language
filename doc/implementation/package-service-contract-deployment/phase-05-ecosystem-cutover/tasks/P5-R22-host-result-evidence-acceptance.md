# P5-R22：Host Result Evidence Acceptance

## 角色、输入与边界

使用未参与 D30、F22A、旧 R21/R21C、I16/G16 或其它验收的全新只读 Agent。唯一权威业务语义来自
`doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14与 P5-F04/F04A 的唯一
checked-in Host test、唯一 assertion及最终值；实现合同为 P5-F22A。输入为包含本合同、F22A result、更新后的 I16 与
G16E/F04 receive 前置合同的 exact clean integration candidate，以及同候选 F22A cheap combined 证据。

只给 `R22 PASS` 或 `R22 FAIL`，不给 I16、G16、F04 或阶段 verdict；不修改/提交，不运行 Cargo、dependency install、
I16、真实 source suite/Host/full/runtime 或 stable。I16 是 R22 PASS 后继，不是本验收前置。

## 验收矩阵

逐项静态核对并以 production owner 做窄 black-box 验证：

1. `platform-source-probe-host-evidence.mjs` 是 fixture guard、Host result/PASS identity、final-value projection唯一
   owner；`platform-source-probe-evidence.mjs`只薄 re-export，不存在第二 success parser。
2. production Gate 不硬编码 `main.__test`、旧错误 `main.test.skiff` 或其它 runtime module spelling。当前真实形态
   `PASS main.__test::provider observes helper mutation`通过；另一个语法合法 module 作为唯一目标也必须通过。
3. 目标只允许出现在 stdout 的 Host segment，即第一条 exact std result之后、第二条 exact Host result之前；位于std
   前、第二条 result 后或仅 stderr 均不能成立。
4. exact test name必须唯一。wrong、missing、same/cross-module duplicate、空/非法/non-ASCII module、malformed
   delimiter、oversized identity全部 fail closed；unrelated PASS不能冒充。
5. `11/11`后`1/1`的顺序与数量、command code/signal/error、owned process/port evidence均为硬条件；任一缺失时
   `sourceSuite:null`。
6. fixture仍为三行唯一test与唯一可达
   `assert root.main.run() == "provider-observed-helper-mutated"`；actual observed PASS与该 assertion共同成立后才派生
   finalValue。`observedPassLine`和`finalValueEvidence.passLine`保存实际有界行。
7. unexpected PASS只保留`PASS <unexpected sha256:<64hex>>`，hash必须等于原行SHA-256，不泄漏原文；stdout/stderr/
   output hashes和v6 bounded diagnostics接线不退化。
8. v6 Host attempt字段集合不增不减；`expectedPassLine`仅为
   `PASS <runtime-module-path>::provider observes helper mutation`协议描述，不是旧literal。
9. throw/nonzero/signal、primary-before-cleanup、wrong result、fixture alternate assertion仍fail closed；diagnostic owner的
   routine-line过滤不是第二套success parser。
10. F22A未改 runner/discovery/fixture、shared-target orchestration、dependency/startup、Router/Runtime、schema、
    manifest/lock或公共业务语义；P27R/R21C聚焦证据的相应blobs保持bit-identical。

现有 checked-in 测试之外，reviewer须用不写文件的内存 black-box probe直接调用production owner，额外覆盖：

- 一个非`main.__test`的合法module作为唯一Host目标仍PASS；
- 目标位于第二条result之后时FAIL；
- 目标只在stderr时FAIL。

不得为 probe 复制 parser/identity grammar或修改测试文件。

## 聚焦命令与交付

```bash
node --test scripts/tests/platform-source-probe-host-evidence.test.mjs
node --test \
  --test-name-pattern 'combined and full modes remain disjoint command-double orchestrations|full Host attempt records nonzero, signal, and exact parse failures|bounded diagnostic retains phase|unreachable expected assertion' \
  scripts/tests/platform-source-shared-target-probe.test.mjs
node --check scripts/lib/platform-source-probe-host-evidence.mjs
node --check scripts/lib/platform-source-probe-evidence.mjs
node --check scripts/lib/platform-source-shared-target-probe.mjs
git diff --check
```

要求 matched>0，并回报 exact candidate/tree/lock、clean before/after、逐条PASS/FAIL、内存probe三项、反搜旧literal/
module oracle/第二parser、测试计数、extra-review与blocking findings。PASS只解锁同一candidate的replacement I16 v6；
candidate、Host evidence owner/fixture/runner formatter/discovery、orchestration/schema/lock任一变化都会使R22失效。
