# P5-F345 Service error cross-layer convergence

状态：Completed（C1，test-only；实现与合流验证通过，等待 F346 独立验收）。

## 直接父节点

- C1/W1/S1 owner、正负探针与证据失效边界：
  `P5-F333-wire-observability-delta-audit-result.md`
- H/R/T实现结果：
  - `P5-F340-service-error-host-consumer-result.md`
  - `P5-F341-service-error-router-consumer-result.md`
  - `P5-F342-restricted-telemetry-storage-query-result.md`
- 合流测试闭环：
  - `P5-F343-host-error-model-test-fixture-closure-result.md`
  - `P5-F344-router-bootstrap-test-determinism-result.md`

父节点已沿引用链连接唯一权威设计。本任务只建立同一场景事实驱动的跨层证据，不修改 production、
shared DTO/schema/corpus或既有 owner测试。

## 起点与场景

- 起点 commit：`335af586c132ffa74e04d5a58b515cf717c9d6ae`
- 起点 tree：`86e97cc0be2bb5e0891b0a27668464687c580187`
- 复用 C0 shared corpus
  `runtime/transport/testdata/service-error-response-v2.json`中的
  `internal-fixed-service-error`作为唯一 wire bytes owner，不复制一份 service error envelope。
- 新建一个跨层场景 fixture，只拥有下列测试事实：
  - 对上述 corpus case的引用；
  - traceId/errorId；
  -一个 private sentinel；
  - A/B/C三个 service/activation/operation/source/local-stack期望；
  - external safe message期望。
- 场景语义：
  - A首次把不可公开/不可序列化错误转为`std.service.InternalError`；
  - B、C均未捕获，继续转发同一个 fixed error bytes和同一 trace/errorId；
  - A/B/C各自只有本地 stack；转发 hop可有脱敏 remote-boundary frame，不继承 callee local frame；
  - operational事件可按 correlation查询但不含 stack；
  - restricted事件三条，普通查询不可见，store-only读取可见经脱敏后的 stack结构；
  - HTTP/WS只含稳定安全信息与 correlation，不含 private sentinel/source/function/stack。

## 写入边界

只允许新增：

- `testdata/package-service-contract-deployment/service-error-convergence.json`；
- `runtime/host/tests/p5_f345_service_error_convergence.rs`；
- `router/tests/service-error-cross-layer-convergence.test.ts`；
- `telemetry/tests/service-error-cross-layer-convergence.test.ts`；
- `P5-F345-service-error-cross-layer-convergence-result.md`。

不得修改任何现有 production、test、fixture/corpus、Cargo/package/lockfile、设计、父 task/result。
若 public test seam不足，停止并返回精确 blocker；不得在 C1增加 production/test-support API。

## 必须建立的同场景证据

### Rust Host / wire

1. 从 C0 corpus按名字读取 Internal fixed case，保留原 UTF-8 bytes（包括原布局），strict decode。
2. typed eval/request carrier→`ResponseEvent::FixedServiceFailure`→Rust v2 frame→dedicated decode→
   outbound carrier，A/B/C三次转发 bytes均完全相等；相同 code/message的 control仍 generic。
3. fixed operational事件使用 corpus trace/errorId，只含有限 kind/cause和预算，不含 stack/sentinel。
4. 通过 F340 production eval telemetry context把场景 fixture的三份 typed diagnostic投影为三条
   restricted event；三者 owner/source/local stack不同但 correlation相同。该测试验证 host projection，
   不伪称手工 diagnostic本身替代 eval真实产生证据。
5. 同时运行已有真实 eval selector：
   - `restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks`；
   - `service_error_channel_contract_operation_restricted_service_diagnostic_real_lanes`。
   前者证明真实多跳 export/forward exact bytes与逐跳 local stack；后者证明顶层
   ContractOperation/ingress export。result必须把两条与 C1 host projection证据明确组合，不能只跑手工值。

### Router

1. 读取同一个 C0 corpus case与场景 fixture；C0 strict seam返回原 payload对象和相同 correlation。
2. `unaryFrame`/fixed mapper继续不重编码，matching generic control不升级。
3. fixed HTTP payload与 WebSocket external message只含场景 safe message、trace/errorId；private sentinel、
   sourceId/source/path/function/frames/stack/encoded payload全部不存在。
4. 运行 F341实际 Assembly HTTP/WS与 endpoint/dispatcher selectors，不能只测 error class。

### Telemetry

1. 读取同一场景 fixture，插入一条 operational和 A/B/C三条 restricted事件，correlation一致。
2. `queryLogs/queryTrace/queryTraces`及真实 `/logs`、`/traces`、`/traces/:id`只返回 operational；
   top-level errorId可关联。
3. store-only restricted reader按 traceId/errorId返回恰好三条，保留每 hop source/stack结构并遮罩
   private sentinel；不得有公开 restricted route。

## 负向反搜

在合流候选上证明：

- fixed producer/consumer不命中
  `response_error_to_telemetry_map|RuntimeResponseError|runtimeErrorStatus`；
-没有 v1 service `response.error` producer/reader/fallback；
- external HTTP/WS输出不含
  `sourceId|sourceFrame|sourceFrames|frames|stack|function|path|private sentinel`；
- ordinary telemetry query filter均显式 operational，restricted reader必须 correlation selector；
- middle hop没有重新生成 InternalError或 correlation。

允许 control/pre-ingress owner仍命中 generic error helpers；result必须区分合法命中与 fixed路径。

## 验证

先列出所有新增和复用 selector且确认非零。至少运行：

```bash
cargo test -p skiff-runtime-eval <两个真实 eval selector>
cargo test -p skiff-runtime-host --test p5_f345_service_error_convergence
pnpm --filter @skiff/router exec vitest run \
  tests/service-error-cross-layer-convergence.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-websocket-gateway.test.ts \
  tests/assembly-runtime-endpoint.test.ts
pnpm --filter @skiff/telemetry test
pnpm --filter @skiff/router run type-check
pnpm --filter @skiff/telemetry run type-check
git diff --check
```

可以使用已存在依赖的临时 symlink，但验证后必须删除。不得运行 workspace/root/stable/live，不 push。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f345-service-error-convergence`
- branch：`codex/p5-f345-service-error-convergence`
- 新的一次性开发 Agent；
- result写明同一 fixture如何连接真实 eval、Host投影、wire bytes、Router gateway与Telemetry query，
  每条 W1/S1 probe的证据及任何不能形成真实链路的限制；
- 提交并返回 commit，不修改 task状态，不承接后续验收。
