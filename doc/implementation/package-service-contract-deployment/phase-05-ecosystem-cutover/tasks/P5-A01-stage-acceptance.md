# P5-A01：Independent Stage Acceptance

## 角色与精确输入

未参与Phase 05计划、实现、修复或T13 gate的只读最终验收Agent。完整阅读权威设计
§1–§15、`phase-plan.md`、全部P5任务合同与verdict、T13覆盖矩阵/ledger，并检查三个repo
冻结exact commits/trees。

不得修改文件、创建commit、顺手修复或重跑仍有效的昂贵gate。只运行风险所需的聚焦
抽查，按设计终态而不是开发总结判断。

## 必验完成态

- 四对象是唯一production domain artifacts，authoring/storage/release/control不重建aggregate或
  legacy/dual/fallback。
- contract-first、package independent compile、deployment exact validation、complete assembly activation及
  InProcessBoundary全链成立，每层owner/identity分离。
- active pointer CAS/atomic reload/failure rollback/request generation pin/multi-replica exact assembly动态正负例成立，
  不承诺service级隔离或扩缩。
- Host ingress是唯一外部选择语义，request path无artifact I/O，旧service/version/build/display
  selectors不可达。
- test-runner、fixtures、`skiff-packages`、registry/platform、Codex、AIHub、Agine/clients全部使用
  canonical flow，contract-owned schema/state/config/secret binding完整。
- ecosystem checker对production owners与mutation/omission可信，全局legacy命中有replacement/删除证明。
- non-live full verify、isolated registry、two-replica、provider/list/chat final-result self-tests均锚定同一
  冻结候选；stable live尚未执行且已由V01唯一owner明确承接。
- 三仓commit/预期merge/worktree/branch收尾计划与未经pusher授权符合repo/workspace规则；当前尚未合入main。

## 输出

第一行 `PASS` 或 `FAIL`。列blocking issues、non-blocking follow-up、证据命令、动态缺口与残余
风险，并说明每条阶段标准由何真实入口/证据覆盖。只有PASS才允许主Agent完成阶段
结果文档草案并将三仓各自一次合入main；worktree/临时分支必须保留到V01 PASS。
