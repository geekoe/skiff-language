# P5-F339 Response error schema blocker reacceptance result

状态：`PASS`（F337 唯一 blocker B1 已关闭；无 blocking；未修改 task 状态，未 push，未承接
H/R/T）。

## 候选与复验边界

- worktree：`/Users/geek/workspace/skiff-p5-f339-response-error-reacceptance`
- branch：`codex/p5-f339-response-error-reacceptance`
- F338 implementation commit：
  `3c69f12b9f81fe29827f3f5d43c489c6bee2cd22`
- F338 implementation tree：
  `7732c2b2920042712ce1d7a9b8b2aca32ed8ede7`
- integration merge commit：
  `79fccb88acdc8c85aafff3c88ea3d1b2532c46d0`
- integration merge tree：
  `7732c2b2920042712ce1d7a9b8b2aca32ed8ede7`
- 本轮起始 HEAD：
  `783f8efbf4549903bb1054ffaef3c9e073c1ff60`

implementation 与 integration merge 的 tree 完全相同。复验只读取 production、tests、fixture
及直接父节点；唯一写入是本 result。由提出 F337 B1 的原验收 reviewer 执行本次同一精确 blocker
复验，符合任务注明的 reviewer 例外。

## 独立判断

### 1. Production declarative schema 是 exact discriminated union

`runtimeFrameHeaderSchemas['response.error']`现在引用两个 object branch 的`oneOf`：

- `fixedService`要求且只允许
  `schemaVersion/type/requestId/errorKind`；`errorKind`只能为`fixedService`，
  `additionalProperties:false`因此同时禁止 generic `error`与其它 extra field。
- `control`要求且只允许
  `schemaVersion/type/requestId/errorKind/error`；`errorKind`只能为`control`。
  nested `error`要求`code/message`，只允许`code/message/status/details`；
  code/message必须包含非空白字符，status必须是400–599的整数。
- 两个 branch 均固定`skiff-runtime-frame-v2`、`response.error`，并要求包含非空白字符的
  requestId。互斥的`errorKind` enum保证合法值只命中一个 branch。

这已直接消除原 optional-property bag 的两个反例：
`fixed-carries-generic-error`不能命中 fixed branch，`control-missing-error`不能命中 control
branch。

### 2. Schema 类型扩张与 production 调用面受控

`ProtocolEnvelopeSchema`只从单一 object扩成
`ProtocolEnvelopeObjectSchema | ProtocolEnvelopeOneOfSchema`；property schema只增加本 contract
所需的`minLength/pattern/minimum/maximum`可选约束。F338 diff没有改写其它 frame schema。

production中读取 envelope schema properties 的唯一调用面通过
`protocolEnvelopeSchemaPropertyNames()`完整区分 object / `oneOf`，遍历全部 branch并去重字段名，
不会对 union直接读取不存在的`properties`。该 helper没有 branch遗漏、TypeScript `any`、宽泛
cast或异常路径；F338 production diff新增的`type: 'any'`只是既有 schema vocabulary中的 details
值语义，不是 TypeScript escape hatch。

### 3. 测试实际解释 production schema

聚焦测试直接读取 production
`runtimeFrameHeaderSchemas['response.error']`，test-side evaluator递归解释：

- `oneOf`且要求恰好一个 branch匹配；
- object `required`与`additionalProperties`；
- type、enum、string `minLength/pattern`；
- integer、`minimum/maximum`；
- nested object与 array items。

它没有调用 manual validator来产生 schema结论，也不是只检查 schema对象是否存在字段。
既有`validateResponseErrorFrame(header, payloadBytes)`仅用于随后验证完整 header+payload seam。

### 4. 同一份 4/30 corpus 完整且互斥

读取的 shared
`runtime/transport/testdata/service-error-response-v2.json`包含4个 valid case和30个 invalid case。
schema测试显式固定13个 header-invalid与17个 payload-only invalid，并断言两个集合合并排序后
精确等于 corpus全部 invalid case name：

- 4个 valid header全部由 declarative schema接受；
- 13个 header-invalid全部被 schema拒绝，其中包含
  `fixed-carries-generic-error`与`control-missing-error`；
- 17个 payload-only case全部先由 schema接受 header，再由既有完整 seam拒绝；
- 13/17 size断言与全名集合等值断言使 corpus未来增删 case或漏分类时直接失败。

既有 seam selector还验证4个 valid case全部成功，并以引用相等断言返回原
`Uint8Array`，没有复制、stringify或重编码 payload bytes。

### 5. Surface parity 与未触碰边界

F338 implementation 相对其起点只包含：

```text
A  .../P5-F338-response-error-declarative-schema-fix-result.md
M  router/src/protocol/runtimeProtocol.ts
M  router/tests/protocol.test.ts
```

wire interface仍是`FixedServiceResponseErrorFrameHeader |
ControlResponseErrorFrameHeader`；manual validator与 header+payload seam未改，其 fixed/control
分支、nonblank约束和 status范围与新 declarative schema一致。F338没有修改
`router/src/protocol/envelope.ts`、shared corpus、Rust runtime、telemetry、Router consumer或
F337其它已 PASS owner，因此没有推翻 F337 的 fixed carrier、exact bytes、telemetry与 dependency
结论。

## Selector 与验证

先枚举并确认两个目标 selector均非零：

```text
tests/protocol.test.ts > runtime protocol fixtures and schemas >
  evaluates the response.error declarative oneOf against the shared header corpus
tests/protocol.test.ts > runtime protocol fixtures and schemas >
  validates the shared service_error_response_v2 corpus without changing payload bytes
```

随后分别运行，不扩大为完整 Router suite：

```text
vitest run tests/protocol.test.ts \
  -t 'evaluates the response.error declarative oneOf against the shared header corpus'
  1 file passed
  1 test passed | 46 skipped

vitest run tests/protocol.test.ts \
  -t 'validates the shared service_error_response_v2 corpus without changing payload bytes'
  1 file passed
  1 test passed | 46 skipped

git diff --check
  PASS
```

worktree没有自有`node_modules`；枚举与运行时只临时链接主 Skiff worktree的现成 Router依赖，
Vitest root与被测文件仍是本候选 worktree。链接随后删除，production/tests/fixtures保持只读。
没有运行完整 Router、workspace/root、stable或 live验证。

## Blocking 与 gate

Blocking：无。

F337 的唯一 blocker B1 已被 exact discriminated schema、真实 production-schema evaluator及同一
4/30 shared corpus共同关闭。因此 C0 contract可冻结，H/R/T fan-out可解除阻塞并由各自 owner继续。
本结论不表示 H/R/T、W2-W、A6或 Phase 5完成，也未在本任务内承接这些工作。
