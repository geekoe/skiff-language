# P5-D50：Canonical Unary Stall Audit

DAG节点D50，依赖I02D在冻结production commit
`42f322364f46f0be9350f4535ff492a562e73ae1`、tree
`9692c132cd07b06a1935772d63deea1ec86467c3`的FAIL。权威设计为
`doc/architecture/package-service-contract-deployment.md`及其引用的spawn/recoverable条款。

三个全新只读分片可并行：

- D50A：从I02D原始ledger与Runtime/Router实现重建首次unary时间线，定位最后进展、挂起资源与缺失终止事件。
- D50B：审计canonical spawn submit从eval/host/actor control到continuation wake的production路径，核对typed
  submitted receipt、worker execution要求及不存在的consumer。
- D50C：审计I02 fixture/driver预期编排，核对谁应发送或消费receipt、WebSocket/marker关系，以及当前direct测试
  是否把未实现的worker行为替换为脚本内模拟。

分片只返回代码/日志事实、设计追溯、遮挡关系、唯一owner候选、最小非E2E诊断探针与是否命中既有D46设计缺口。
禁止编辑、提交、重跑I02/R05/instance/stable/full gate，不作I02/R02 verdict。汇总后若为设计缺口必须暂停受影响
DAG并给用户最小选择；若设计已明确则批量更新唯一修复owner。
