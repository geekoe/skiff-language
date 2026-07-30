# P5-F384 Test assembly gateway control-plane audit

状态：Ready（只读）。

## 直接父节点

- `P5-F381-registry-current-generation-storage-blocker.md`
- `P5-F359-http-gateway-request-protocol-result.md`
- `P5-F365-host-http-gateway-admission-wire-result.md`

本节点只审计HTTP test/control fixture如何从旧ContractOperation ingress迁移到canonical deployment gateway
entry。WebSocket业务消息路由不在范围内，不得用其未决设计阻塞HTTP test迁移。

## 必须追踪

从以下producer/consumer逐跳列出旧字段、canonical替代字段、identity owner与调用顺序：

- `test-runner/src/package_test_assembly.rs`
- `test-runner/src/bin/ecosystem_smoke_fixture.rs`
- test-runner runtime execution/request header owner
- Router `assemblyControlPlane.ts`及相邻assembly request parser
- F359 HTTP gateway request header
- F365 Host admission/wire。

至少回答：

1. 两个Rust fixture分别仍在哪里制造`ContractOperationId`、operation binding或旧ingress；
2. Router control-plane仍在哪里读取`contractOperationId`或旧`testEffectDoubles`；
3. package-test synthetic gateway entry应采用什么exact selector、mode、adapter arguments、return shape与
   gateway identity；
4. ecosystem smoke fixture需要几个gateway entry，如何从既有operation调用映射，是否可共享同一
   canonical builder；
5. `testEffectsEnabled`如何传递；inline setup已经拥有test doubles时，wire上是否应完全移除
   `testEffectDoubles`；
6. 应先做一个Router shared checkpoint再做test-runner consumer，还是可在单一owner安全完成；列出文件
   重叠和最小DAG。

## 不变量

- production HTTP request必须继续`testEffectsEnabled = false`；
- 只有`kind: test`的隔离test path可启用测试effects；
- 不恢复旧contract-root ingress，不伪造service operation；
- synthetic package-test可以使用零operation contract，但必须有reference-closed deployment
  gateway entry；
- assembly generation、gateway entry identity、method/path/selector都必须由canonical artifact/header
  提供，不能靠Router重算或猜测；
- test doubles保持编译到inline setup，不重新加入wire JSON；
- 不改WebSocket协议。

## 交付

Skiff production只读；允许运行聚焦现有测试或temporary fixture，但不修改文件、不操作stable/live、不派
子Agent。

在本任务worktree写
`P5-F384-test-assembly-gateway-control-plane-audit-result.md`，给出：

- exact调用链和所有旧字段命中；
- frozen request/response示例；
- Router与test-runner分节点文件边界；
- 每个后继的正负测试与运行命令；
- 是否需要用户决策。

result本地commit，worktree clean；不merge/rebase/push。
