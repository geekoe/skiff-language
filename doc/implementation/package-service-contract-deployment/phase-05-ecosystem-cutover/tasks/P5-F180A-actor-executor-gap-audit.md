# P5-F180A：Actor Executor Gap Audit

状态：Ready

## 直接父任务

- `P5-F179-actor-registry-surface-and-control-result.md`

## 目标

只读审计当前compiler→runtime→router actor method链与
`doc/architecture/actor-model.md`目标态差距，形成可实施DAG。不得修改production代码。

## 必须回答

- `hub.submitOp(...)`从source typing、lowering、File IR、linker、native/control wire到owner runtime
  method dispatch的每一段当前状态。
- actor bootstrap从registry payload到owner runtime materialization及字段存储的当前状态。
- 同一实例单线程executor、多个方法在suspension point交错、恢复epoch检查、字段访问隔离是否已有owner。
- getOrCreate/replace/find/remove与method call如何共享logical identity、epoch、ABI和implementation identity。
- idle TTL、runtime crash恢复、upgrading admission关闭、安全点退出、旧implementation拒绝的现有能力与缺口。
- 哪些缺口可并行，哪些需要shared checkpoint；给出精确文件owner、测试边界和任务顺序。
- 若权威文档仍缺会改变公共语义的决策，必须列为阻断，不得自行补协议。

## 验证

- 报告必须引用当前production符号/文件证据；
- 明确区分“已有完整路径”“只有DTO/fixture”“完全缺失”；
- 写`P5-F180A-actor-executor-gap-audit-result.md`并独立提交，不修改production。
