# P5-F45B：Actor Control Activation Wire

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§10、§12及§14。

DAG节点F45B，依赖D45用户决策。它是canonical actor/spawn activation identity的唯一shared checkpoint；完成后扇出
F45C Runtime与F45D Router，不实现两端production consumer或I02 probe。

独占写入shared DTO/codec/parity表面：

- Rust canonical ActivationIdentity representation及actor put/find/remove、spawn submit/claim/complete/renew/fail control
  request DTO；
- `runtime/transport` control mapper/frame codec与聚焦tests；
- Router `protocol/envelope.ts`、`runtimeProtocol.ts`对应frame header/schema/validator及protocol tests；
- 为跨crate依赖方向所需的最小Cargo manifest，Cargo.lock不得提交机械变化。

所有canonical actor/spawn request frame必须携带完整structured ActivationIdentity：
assembly identity、generation、runtime replica、deployment revision。缺失、unknown field、非法identity、tuple部分匹配或
legacy string activationIdentity必须fail closed。response/request correlation与payload framing不变；不得在本checkpoint
加入Router registry/snapshot授权或Runtime current-context填充。

开发owner运行：

```bash
cargo test --locked -p skiff-runtime-capability-context -p skiff-runtime-transport
pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
pnpm --filter @skiff/router type-check
git diff --check
```

必须提供Rust/TS同一golden corpus及bad/missing/unknown/partial变异；反向搜索证明actor/spawn canonical header不再接受旧
optional string identity。禁止修改Router registry/actor control consumer、Runtime host/eval consumer、scripts/I02、
fixtures、release/activation语义或四对象；禁止真实probe/instance/stable/full gate。

独立worktree/branch从当前integration checkpoint创建，5分钟内开始修改，否则`TASK_NOT_EXECUTABLE`。提交并返回
自验收矩阵，不push、不merge main。Cargo/lock变化必须单独报告，不得混入提交。
