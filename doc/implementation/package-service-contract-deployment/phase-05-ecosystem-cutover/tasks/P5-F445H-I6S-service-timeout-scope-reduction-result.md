# P5-F445H-I6S service timeout scope reduction result

状态：`DESIGN_UPDATED / FIRST_VERSION_SCOPE_REDUCED`。

## 直接父节点

- `P5-F445H-I6R-current-scope-refresh-preflight-result.md`

用户于2026-07-28选择预检§6.2的简化方案。本节点把该决策写回唯一权威设计及runtime reference，
不修改production或tests。

## 冻结结论

1. 第一版service call没有独立的consumer dependency timeout。
2. 第一版service call没有独立的callee operation timeout或callee default。
3. Service call继承调用点current execution deadline；它已经包含caller request deadline与外层
   `timeout(...)`的收紧。
4. 调用方需要更短预算时显式使用`timeout(...)`。
5. Operation已经存在的primitive timeout仍与current execution deadline取最早者。
6. Deployment `policy.timeoutMs`继续只属于external ingress/request policy，不复用为内部service
   call默认值。
7. 不恢复legacy `ServiceTimeoutConfig`，不新增dependency/artifact/assembly字段，也不提供兼容路径。

## 更新的权威文档

- `doc/architecture/package-service-contract-deployment.md`
- `doc/reference/runtime.md`

## 对 I6 DAG 的影响

- 删除I6 service schema/compiler/loader/production节点；
- 复用E4R已通过的canonical service current-scope owner；
- I6-J的service case只需证明caller current scope传播，并反向确认没有legacy relay、
  dependency/callee timeout字段或deployment policy复用；
- I6-J与独立I6 acceptance不再受service timeout设计阻塞。

I6-A、I6-B、I6-C、I6-D的实现范围不变。本节点不表示这些实现已经完成。

## 验证

- `rg`确认公开权威文档不再把consumer dependency timeout或callee operation timeout列为第一版
  deadline来源；
- `git diff --check`通过；
- 未运行Cargo、测试、network、stable/live或MongoDB。
