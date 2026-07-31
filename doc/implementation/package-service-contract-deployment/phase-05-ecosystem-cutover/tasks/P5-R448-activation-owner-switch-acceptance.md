PASS

# P5-R448 Activation Owner Switch Acceptance

## Role

在F448 exact integration candidate上做独立只读验收。验收Agent不修代码，不重复定义context字段，也不把
“service call能返回”当作owner正确的替代证据。

## Authority

- [`P5-F448-activation-owner-switch-atomic-rebind.md`](P5-F448-activation-owner-switch-atomic-rebind.md)
- [`package-service-contract-deployment.md`](../../../../architecture/package-service-contract-deployment.md)
  §6.2
- [`runtime-deployment-topology.md`](../../../../architecture/runtime-deployment-topology.md)
  “Activation execution owner switch”

## Acceptance Matrix

| 场景 | 必须观察到 |
| --- | --- |
| caller调用同generation provider | config、DB、file、actor capability、spawn、WebSocket、telemetry与service binding全部来自provider deployment |
| provider返回caller | deadline、内部停止、time、request generation/lifecycle、trace/error、transport request identity、stream/test effect/case capability及heap limits保持同一request事实 |
| 参数、返回、错误与callback roundtrip | provider使用fresh heap；所有boundary value重新materialize；caller mutable root/call frame/slot不进入provider |
| caller在actor method中发起service call | caller `ActorExecutionFrame`不进入provider；只有实际Pending才按既有规则释放caller executor |
| payload含内部`ActorRef` | 显式actor route owner不被rebind改写；当前provider actor capability仍来自provider deployment |
| provider读取Package静态资源 | 按provider current callable的`RuntimeExecutionProjection`读取；不存在activation-owned资源副本 |
| callback进入capability owner | 使用同一rebinder切回callback owner，返回后恢复receiver context；没有独立手写切换 |
| active与draining generation并存 | 旧request/service stream命中旧exact assembly/snapshot/deployment context，新request命中新active context |
| escaping service stream跨generation切换 | 后续item、callback和terminal继续使用旧generation，stream释放后才释放pin |
| exact target missing/duplicate/mismatch | provider执行前fail closed，source context不变；不查service latest、active pointer或ambient context |
| 普通continuation、Package direct、actor resume、spawn start、native helper | 不调用rebinder；分别恢复或创建其已验证context |

## Static Gates

- `ActivationExecutionContextRebinder`只有service provider entry与callback owner entry两个production caller；
- `ActiveAssemblyContextSet`以exact pair/generation/deployment为key，并保留有pin的draining entry；
- deployment-scoped与request-scoped字段清单各有一个canonical projection owner，没有散落的optional
  overwrite；
- 不存在latest/current service lookup fallback、thread-local owner、partial context mutation或caller heap
  直传；
- Runtime聚焦测试和必要combined gate来自同一exact commit/tree，Rust编译只由integration gate owner执行
  一次；
- `git diff --check`通过。

第一行输出`PASS`或`FAIL`。FAIL必须给出exact commit/tree、失败场景、唯一生产owner及最小修复边界；不得在
验收worktree修改候选。R448 PASS只解除R446 owner-switch前置，不单独宣称Phase 05完成。

## Result Record

验收锚定commit `6dcf8d4906ce06f2576b9577273306fc3fbbeef7` / tree
`98e2e9c774c0e898cf58b22045512487ebb36185`。Eval library `418/418`、三个integration binary
`15/15`；Host library `339/339`、三个integration binary `11/11`。owner-rebind聚焦Eval `2/2`、
Host `4/4`，typed execution端到端`18/18`，config owner、DB identity、generation pin、spawn、
Package direct、test effects、heap与static resource矩阵`8/8`。

Runtime DAG、artifact/execution/eval error、artifact identity、格式与diff检查均PASS。后续真实Agine
`aihub/deepseek-v4-flash` chat smoke和双Host验收都通过，证明provider stream与callback在stable链路完成。
结论：R448 **PASS**。
