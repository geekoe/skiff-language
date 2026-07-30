# P5-F42：Shared RuntimePayload Test Codec

权威设计为
`doc/architecture/package-service-contract-deployment.md` §6.2、§7、§12及§14。

DAG节点F42，依赖D42 COMPLETE，与F43并行；完成后与F43共同解除F44。目标是把现有
`router/tests/helpers/runtimePayloadCodec.ts`的generic RuntimePayload JS test mirror迁移为scripts与Router tests都可
消费的唯一owner，禁止产生第二个`SKPV` parser。Rust `runtime/boundary/src/binary.rs`仍是production owner。

独占写入：

- 新增`scripts/lib/runtime-payload-codec.mjs`及必要类型声明；
- 新增`scripts/tests/runtime-payload-codec.test.mjs`；
- `router/tests/helpers/runtimePayloadCodec.ts`只保留Router manifest schema adapter与对shared owner的薄re-export；
- 为保持Router tests/type-check接线所必需的最小protocol test/import配置。

依赖方向固定为`Router tests → shared test codec ← lifecycle scripts`；production Router/Runtime不得依赖JS helper。
迁移generic implementation，不复制或保留第二份parser。codec保持现有magic/version/schema tag、nullable/union、
early EOF、trailing bytes、finite/integer及u32 length语义；调用者输入预算由F44承担，不在本任务发明公共limit。

开发owner运行：

```bash
node --check scripts/lib/runtime-payload-codec.mjs
node --test scripts/tests/runtime-payload-codec.test.mjs
pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
pnpm --filter @skiff/router type-check
```

至少包含独立checked-in或手工构造golden：`SKPV v2 + string tag + u32 length + B marker`，以及bad magic、bad
version、wrong tag、early EOF、trailing bytes；不能只用同一codec encode→decode round-trip。

禁止修改Rust codec、production Router/Runtime、fixtures、generation harness、release/activation/四对象或公共ABI；
禁止真实transcript/instance/stable/完整gate。独立worktree/branch，从当前integration checkpoint创建，5分钟内开始实际
修改，否则返回`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
