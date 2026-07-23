# P5-F29A：Router WebSocket Reject Strictness

权威设计为
`doc/architecture/package-service-contract-deployment.md` §5中operation descriptor必须逐项精确匹配、§6.2的service
dispatcher边界，以及§14中identity/descriptor错误必须fail closed且runtime不得从raw JSON补事实的条款。

DAG节点F29A，依赖R24在exact candidate `51558c928778f8004361595f3dcd2ab5b79ea53e`首次FAIL；完成后只解除原R24
reviewer对同一精确blocker的窄复验。风险高，验收分组为F05 WebSocket response trust boundary。R25/R27/I28/R30正路径证据
仍有效，不重跑real smoke。

写入边界仅：

- `router/src/protocol/runtimeProtocol.ts`；
- `router/tests/runtime-protocol-websocket-response.test.ts`；
- 如直接测试必须导入共享response corpus，只允许最小test-only loader，不修改corpus或Assembly gateway production。

完成标准：

- reject variant的`code`与`reason`都必须存在且类型/范围合法；missing/extra/错误类型在Router protocol boundary直接拒绝；
- TS直接测试消费
  `cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json`中的全部valid/invalid cases，至少明确覆盖
  `reject code missing`与`reject reason missing`；
- invalid reject不能到dispatcher/gateway default 403路径；accept、receive、zero-byte Context与现有mutation语义不回归；
- 不改Rust/canonical ABI、Assembly gateway、dispatcher、business ABI、四对象或generation lifecycle。

快速验证命令：

```bash
pnpm --dir router exec vitest run tests/runtime-protocol-websocket-response.test.ts
pnpm --filter @skiff/router type-check
git diff --check
```

测试数必须非零。禁止real smoke、combined/full/I16/Host/stable。一个clean commit，不merge/push。Router runtime response
schema/codec或共享corpus变化会使证据失效；完成后仍只是Implementation Checkpoint。
