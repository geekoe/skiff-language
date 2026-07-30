# P5-I32：R05 Unary Repair Combined

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点I32，依赖F41A合流到integration commit
`8c832b44a49b31da393064ab2c6c7d432db70274`。这是新预验收周期的唯一cheap combined owner，只闭合R05失败的
outbound unary wire、成功B marker、404 diagnostic与完整orchestration direct接线；不作R05/R02/Phase verdict。

全新只读Agent先确认exact commit/tree、Cargo.lock及状态，然后在合流状态只运行一次：

```bash
node --test scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs
```

必须确认默认真实unary client而非fixed-200 fake经过动态本地HTTP server，观察receipt-owned method/path/Host；
成功response进入B marker oracle，非200 diagnostic包含实际wire字段与脱敏限长body；orchestration仍覆盖A/B与drain
调用顺序。不得运行真实transcript、fixture combined、旧smoke、Router/runtime/instance、stable或完整gate；不得编辑、
提交或修复。

PASS只解除全新R05A Agent在新candidate上再次运行一次合同冻结的真实命令；FAIL返回精确失败及唯一owner，不重试。
lifecycle real client/test、相关Node HTTP行为、Cargo.lock或checkout source变化会使I32证据失效；I31不因本修复失效。
