# P5-R03：Cross-Repo Ecosystem Pre-Gate Acceptance

## 角色与精确输入

单一只读验收owner，未参与T06–T12/T09E实现。阅读权威设计 §1–§15、`phase-plan.md`、
T06–T12与T09E任务合同/证据，以及主Agent提供的三个repo exact clean commits/trees、combined
integration probe、legacy search ledger及gate preflight计划。

不得修改文件、创建commit、顺手修复或重跑仍有效的昂贵gate。可按风险做不重复的
定向抽查；验收面较宽时可调度互不重叠的只读分片，但只由本owner输出PASS/FAIL。

## 必验完成态

1. 三仓production只有PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly；
   无common aggregate、legacy DTO/reader/writer/converter、dual path/fallback。
2. contract-first publish、provider-less package compile、source-free deployment validation、complete closure assembly activation
   有真实入口及fail-closed负例，不只是手工构造model。
3. active pointer CAS/atomic reload、failed candidate rollback、request generation pin、request-time artifact I/O为零；
   两replica exact same assembly且mutable owner独立。
4. Host ingress区分相同path；Skiff router、platform、Codex、AIHub、Agine clients无service/version/build
   selector选择语义或rewrite fallback。
5. test-runner/package-test/fixtures与`skiff-packages`只用canonical build/store/test；旧test均有replacement/
   deletion disposition。
6. registry四类typed persistence/pointer/audit无PackageUnit fields，account/registry actual services已迁移；
   stale/unauthorized/tampered operations失败不改history。
7. Codex、AIHub、Agine contracts独立、schema自包；AIHub/Agine contract types不冒用package-local nominal
   types，显式wrapper及deployment/state/config/secret binding完整。
8. T09E production `assembly.yml`闭合五个actual deployments；combined probe从canonical authoring/store构建
   该完整Internals assembly，经router/runtime Host ingress到
   最终业务结果；provider/list/chat的stable live执行留给V01，但self-test必须实际断言最终结果。
9. ecosystem checker subject/mutation完整，reference/architecture/runtime/router docs与唯一设计一致。

## 输出

第一行 `PASS` 或 `FAIL`。列blocking issues、non-blocking follow-up、证据命令、尚未执行的
live动态缺口、残余风险，并把每条阶段标准映射到真实入口/证据/owner/exact commit。
PASS后才允许主Agent完成preflight并冻结T13候选。
