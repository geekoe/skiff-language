# P5-F332 Service error channel A5 acceptance

状态：Ready（独立只读验收）。

## 验收输入

- 唯一权威设计：
  `doc/architecture/package-service-contract-deployment.md`，重点6.3；
- runtime/std/static reference：
  - `doc/reference/runtime.md`
  - `doc/reference/std-surface.md`
  - `doc/reference/static-semantics.md`
- owner与真实路径：
  `P5-F319-service-error-channel-delta-audit-result.md`
- R0独立验收：
  `P5-F327-service-error-core-independent-acceptance-result.md`
- R1/R2/R3结果：
  - `P5-F328-service-error-ordinary-ingress-consumer-result.md`
  - `P5-F329-service-error-async-stream-consumer-result.md`
  - `P5-F330-service-error-test-effect-consumer-result.md`
- R4合流探针：
  `P5-F331-service-error-channel-convergence-probe-result.md`

## 精确候选与边界

- production candidate：`5040224ed4729bc8f5608d1c9b7b2cabe7cc9df3`
- R4 evidence commit：`2960cfd95ff0c91a233aad2279e6adc8cf0a2f5f`
- 验收worktree从包含R4 merge/result的integration HEAD创建；必须证明candidate之后runtime production只有
  R4的`#[cfg(test)]`接线，没有额外production变化，否则停止。
- 风险：最高。只验收W2-R/A5 runtime channel；W2-W wire/host/router/telemetry、generic WebSocket决定及
  Phase 5均不在本verdict。

只读production/tests。唯一允许写入
`P5-F332-service-error-channel-a5-acceptance-result.md`并提交。不得修代码、fixture或设计；不得运行完整
eval/workspace/root/stable/live，不push、不承接W2-W。

## 必须独立验收

1. ordinary、async unary、server stream、ingress和service test effect都只调用冻结R0 export/import；
   Package effect保持local；没有lane classifier、raw heap passthrough或message/code推断。
2. B1–B9（含Resource B8a）、S1–S2、T1/T2矩阵在真实linked image/provider/caller heap入口成立；
   尤其B3 unlinked middle hop和B9 Internal三跳。
3. provider heap drop/stream task lifetime/test setup heap之后没有悬空handle或local `TypeAddr`跨boundary；
   selected branch与caller build graph精确。
4. cancel/control和generic legacy error不被误分类；typed fixed stream/response不依赖downcast。
5. 每跳新local stack、安全RemoteBoundary与correlation透明；callee stack/source/private字段不进入fixed bytes。
6. `std.service.InternalError`是普通可catch、可序列化名义值；第一次生成后未处理转发不重复包装。
7. ingress只上交fixed carrier；W2-W尚未接入必须明确列为残余风险，不能因A5 PASS掩盖。
8. production长模块/测试fixture没有因R1–R3继续增长而形成第二owner或明显复制；可读性改进与blocking结构问题
   分开报告。

## 独立证据

不得机械重跑F331全部命令。至少：

- 读production真实调用链并独立反搜legacy/duplicate owner；
- 运行一个ordinary/async合流selector、一个stream/cancel selector、一个service effect selector；
- 独立抽查B3或B9三跳以及一个identity/heap/cancel负例；
- selector必须先列出并非零。

不得运行完整eval；两个generic WebSocket既知失败不属于A5。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f332-error-a5-acceptance`
- branch：`codex/p5-f332-error-a5-acceptance`
- 返回`PASS`/`FAIL`、blocking、non-blocking、独立证据、结构判断与残余风险；
- PASS冻结A5并允许W2-W消费fixed carrier，不代表W2-W/Phase 5；
- result提交，不push、不承接实现。

