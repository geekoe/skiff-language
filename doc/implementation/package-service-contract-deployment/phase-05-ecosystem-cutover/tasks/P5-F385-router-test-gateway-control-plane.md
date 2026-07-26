# P5-F385 Router test gateway control plane

状态：Ready（F384 R0）。

## 直接父节点

- `P5-F384-test-assembly-gateway-control-plane-audit-result.md`

只实现父节点§5、§6 R0与§7.1冻结的HTTP test control合同。WebSocket及general legacy
`RouterControlPlane`不在范围。

## Worktree

- `/Users/geek/workspace/skiff-p5-f385-router-test-gateway-control`
- branch `codex/p5-f385-router-test-gateway-control`
- base：包含本任务的Skiff phase-05 integration。

## Production要求

1. `POST /__skiff/test-dispatch` strict decode父节点§5.1 exact object：
   - top-level只有`kind/routing/mode/httpRequest/payloadBase64/timeoutMs`；
   - `kind`必须为`test`；
   - routing携带canonical assembly identity/generation、gateway entry identity及exact HTTP selector；
   - 不接受`contractOperationId`、deployment/key、`testEffectsEnabled`、`testEffectDoubles`或unknown field；
   - 不lowercase/uppercase/补全/重算任何routing值；
   - Base64必须canonical，timeout必须positive safe integer。
2. exact match active snapshot的assembly/generation/selector/gateway identity/mode/http metadata。
3. 新增显式test-only header/dispatch/registry入口：
   - 复用F359全部canonical header facts；
   - 私有test builder只在strict `kind:test`分支把`testEffectsEnabled`设为true；
   - ordinary production builder仍固定false；
   - ordinary dispatcher继续拒绝true；
   - test-only dispatcher要求true并重复全部canonical validation；
   - 禁止`skipValidation`或通用`allowTestEffects`开关。
4. 成功响应原样返回runtime canonical `response.end` header与opaque payload，不解码业务JSON；control
   parse/match/dispatch错误仍为non-2xx。

## 写入边界

允许：

- `router/src/router/assemblyControlPlane.ts`
- `router/src/router/assemblyHttpGateway.ts`
- `router/src/router/assemblyRuntimeRegistry.ts`
- `router/src/router/runtimeDispatcher.ts`
- 对应direct Router tests/helper。

禁止：

- `router/src/protocol/**`、snapshot/loader DTO；
- general legacy `router/src/router/controlPlane.ts`；
- WebSocket identity/connect/receive/message路径；
- Rust transport/Host/eval/test-runner；
- skiff-packages/Internals/stable/live。

## 验收

正负矩阵严格按父节点§7.1，至少运行：

```bash
pnpm --filter @skiff/router exec vitest run \
  tests/assembly-runtime-endpoint.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/assembly-replica-dispatch.test.ts
pnpm --filter @skiff/router exec tsc --noEmit --pretty false
git diff --check
```

R0-owned HTTP/control production与direct tests必须零type error。若全局typecheck仍只有F378已记录的
WebSocket HTTP-only残留，逐项记录但不得越界修复。

scoped production反搜
`ContractOperationId|contract_operation_id|contractOperationId|testEffectDoubles`为零；明确的reject mutation
可保留在测试。

写`P5-F385-router-test-gateway-control-plane-result.md`，production/tests/result本地commit，worktree
clean；不merge/rebase/push。新Agent执行，不派子Agent。若必须改变F359/F365或WS协议，返回
`TASK_SCOPE_EXPANDED`。
