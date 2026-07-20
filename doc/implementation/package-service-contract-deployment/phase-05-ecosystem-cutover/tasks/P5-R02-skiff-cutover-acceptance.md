# P5-R02：Skiff Consumer Cutover Acceptance

## 角色与精确输入

未参与T02–T05实现的只读批次验收Agent。阅读权威设计 §1–§15、`phase-plan.md`、
T02–T05任务合同与各自证据，检查主Agent给出的exact clean merged commit/tree及combined
integration probe。

不得修改、创建commit、以完整gate代替定向验收或要求T06已删除所有旧type。

## 必验完成态

- compiler/CLI/dev sync/watch/publish只用T01四对象存储与pointer；contract-first/package independent
  compile/deployment/assembly的正负例真实可执行。
- router只用active assembly + Host ingress，legacy selectors不能选择target；reload failure保留旧
  snapshot，多个exact assembly replica可调度。
- runtime startup/control经production resolver/load/link/admit后注册；failed candidate rollback、request
  generation pin、request-time artifact I/O为零。
- test-runner/package-test只产生canonical artifacts，package direct与service boundary仍分开，isolated
  harness不触stable。
- T02–T05只通过T01接口合流，没有重复pointer/wire/path/identity owner、adapter或fallback。
- combined probe至少覆盖tooling产出的fixture被router/runtime加载、Host request到最终业务
  结果、failed reload保留旧generation及test-runner消费同一接口。

## 输出

第一行 `PASS` 或 `FAIL`。按tooling/router/runtime/test infrastructure分别列blocking issues、
non-blocking follow-up、证据命令、动态缺口与残余风险。PASS后exact commit可作外部repo
基线，但T06与external consumers未完成，仍不是final stable candidate。
