# P5-F445H-I7-X Service-scoped ingress independent acceptance

状态：`COMPLETE`

## Parent

- `P5-F445H-I7-D0-service-scoped-ingress-design-result.md`
- `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md`
- `P5-F445H-I7-C-compiler-ingress-consumer-result.md`
- `P5-F445H-I7-L-service-scoped-assembly-consumer-result.md`
- `P5-F445H-I7-R-router-service-scoped-ingress-result.md`
- `P5-F445H-I7-F-test-runner-ingress-fixture-hard-cut-result.md`

## Frozen candidate

```text
commit 57c93c3026a17f8d0c134b80197c294b3a325f52
tree   225cd7e9001fac9a55ce9c5ef89db842402f1f98
```

## Scope

独立复验service-scoped ingress完整纵向合同：

1. canonical generation、旧Host selector与旧代际严格拒绝；
2. compiler `service_config`、HTTP与WebSocket authoring/projection；
3. assembly resolver、loader、linker使用精确deployment作用域；
4. Runtime frame v2携带精确`ServiceDeploymentRef`；
5. Host、runtime request、package-test消费精确deployment；
6. Router typecheck与完整可执行suite；
7. Relay与AIHub可以在同一assembly声明同一`GET /v1/models`，由可信service/version
   header选择不同精确deployment；HTTP Host不参与选择；
8. 同service重复route、缺失/非法header、同坐标多revision、跨deployment替换、旧Host wire
   全部失败关闭；
9. 不同service的同path WebSocket合法，连接固定精确deployment与generation。

## Write boundary

优先零production修改。允许新增本task/result及独立验收测试。若候选只残留旧代际test
fixture/golden，可经主Agent授权后做有界test-only修复；发现production行为问题立即停止并上报。

禁止访问stable/live/network/Mongo/OAuth/browser；禁止push；本任务不直接写integration branch。

## Handoff

结果提交交`/root/phase05_integration_steward`合入Skiff integration；由integration owner清理本
worktree和已合并分支。
