# P5-F338 Response error declarative schema fix result

状态：`PASS`（F338 边界内实现完成；未修改 task 状态，未 push，未承接 H/R/T）。

## 候选与写入边界

- worktree：`/Users/geek/workspace/skiff-p5-f338-response-error-schema`
- branch：`codex/p5-f338-response-error-schema`
- shared production 候选起点：`ce8035d2c83961effb5d5b01b2825a8dd80262f9`
- shared production 候选 tree：`74ee30f5df209aa94c635b1a6a9fae7d09d471f0`
- worktree 起始 HEAD：`a40ece9e405fd834fb283ef9e6650c684c851334`
- worktree 起始 tree：`f03f52dd80ee11765b34685657a866299e6226e1`

production 只修改 `router/src/protocol/runtimeProtocol.ts`，测试只修改
`router/tests/protocol.test.ts`，并新增本 result。没有修改`ResponseErrorFrameHeader` wire
interface、manual validator、header+payload seam、`envelope.ts`、shared corpus、其它 Router consumer、
Rust、telemetry、request/host、lockfile或父 task/result。

## Exact diff

1. `ProtocolEnvelopeSchema`增加最小的 object / exact `oneOf` union 表达；property schema只补充本
   response-error parity所需的`minLength`、`pattern`、`minimum`和`maximum`约束。
2. `runtimeFrameHeaderSchemas['response.error']`不再导出单一 optional-property bag，而是两个完整分支：
   - `fixedService`只允许并要求
     `schemaVersion/type/requestId/errorKind`，`errorKind`固定为`fixedService`，
     `additionalProperties:false`明确禁止 generic `error`和其它字段；
   - `control`只允许并要求
     `schemaVersion/type/requestId/errorKind/error`，`errorKind`固定为`control`；
     nested `error`只允许`code/message/status/details`并要求`code/message`。
3. 两个分支都固定 response-error v2与`type: response.error`；requestId、control code/message使用
   non-empty/non-blank约束，status保持整数400–599。没有增加 v1、fallback、兼容 reader或按值升级 fixed。
4. 既有按 schema 获取 allowed field的 production调用点改为通过 helper遍历全部`oneOf`分支并合并
   字段名；没有 TypeScript `any`、宽泛 cast或跳过分支，其它 frame schema结构不变。
5. protocol test新增通用递归 test-side evaluator，真正解释 production schema的`oneOf`、object、
   required、additionalProperties、type/enum、string、number和array约束。原有 header+payload seam与
   payload reference identity断言保持不变。

## Shared corpus 的 header / payload 边界

同一个`runtime/transport/testdata/service-error-response-v2.json`提供4正/30负：

- 4个合法 header（public/Internal/platform fixed与generic control）全部被声明式 schema接受；
- 13个 header-invalid case全部被声明式 schema拒绝，包括显式的
  `fixed-carries-generic-error`和`control-missing-error`，并覆盖 v1、missing/unknown kind、
  wrong type、header/nested extra、missing/empty requestId、empty code/message与非法 status；
- 17个 payload-only invalid case的 header仍被声明式 schema接受，再由既有
  `validateResponseErrorFrame(header, payloadBytes)`拒绝。它们覆盖 fixed/control payload presence、
  malformed/unknown/extra/missing envelope、identity/correlation、encoded bytes、platform identity和
  Internal nested payload约束。

因此 header schema没有伪装校验 binary payload；manual seam仍拒绝全部30个负例。4个合法 case继续用
引用相等证明返回原`Uint8Array`，没有复制、stringify或重编码 payload bytes。

## Selector 与验证

先运行：

```text
pnpm --filter @skiff/router exec vitest list tests/protocol.test.ts
  47 selectors
```

其中本任务直接 selector 为：

```text
runtime protocol fixtures and schemas >
  evaluates the response.error declarative oneOf against the shared header corpus
runtime protocol fixtures and schemas >
  validates the shared service_error_response_v2 corpus without changing payload bytes
```

最终聚焦测试：

```text
pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
  1 file passed
  47 tests passed
```

Router最小全 package type-check：

```text
pnpm --filter @skiff/router run type-check
  blocked by 9 parent-checkpoint consumer errors
```

精确断点均不在本任务文件：

- `src/router/httpGateway.ts:1007`：既有 telemetry producer缺必填`visibility`；
- `src/router/runtimeDispatcher.ts:649,666,681`：仍把 response-error当作旧 generic
  `error/requestId` header并写 v1；
- `src/router/runtimeEndpoint.ts:604,650`：仍写 v1 generic header并无判别地读取`header.error`；
- `tests/helpers/runtime.ts:323`、
  `tests/runtime-assembly-unary-dispatch.test.ts:329`、
  `tests/runtime-registry-dispatch.test.ts:1129`：仍构造旧 v1 response-error fixture。

按任务要求追加只以本任务文件为 root、使用 Router同一严格 compiler options的证据：

```text
pnpm --filter @skiff/router exec tsc --noEmit \
  --target ES2022 --lib ES2022 --module NodeNext --moduleResolution NodeNext \
  --strict --noUncheckedIndexedAccess --exactOptionalPropertyTypes \
  --esModuleInterop --skipLibCheck \
  src/protocol/runtimeProtocol.ts tests/protocol.test.ts
  PASS

git diff --check
  PASS
```

没有运行完整 Router test suite、workspace/root/stable/live。

## Blocker

F338 边界内 blocker：无。

完整 Router type-check仍由父 checkpoint明确留下的 R/T consumer与旧 fixture hard cut阻塞；这些文件不在
本任务写入边界，不能在 F338越界修复。声明式 schema B1已修复，等待主 Agent合流后用同一 corpus执行组合
探针与精确 blocker复验。
