# P5-F336 Service error wire and telemetry checkpoint

状态：Ready。

## 直接父节点

- 当前 wire/observability owner、C0 范围与 W1/S1 矩阵：
  `P5-F333-wire-observability-delta-audit-result.md`
- 已冻结的 typed restricted diagnostic seam：
  `P5-F335-restricted-service-diagnostic-acceptance-result.md`
- 已冻结的 canonical fixed carrier：
  `P5-F332-service-error-channel-a5-acceptance-result.md`

父节点已连接唯一权威设计。本任务只实现 F333 的 C0 shared protocol checkpoint；不接 request/host、
Router dispatcher/gateway或 telemetry storage/query consumer。

## DAG、候选与互斥边界

- blocked-by：F335 PASS；起点为 worktree 创建时 integration HEAD。
- 完成后解除三个并行 consumer：
  - H：request/host/session；
  - R：Router dispatcher/gateway；
  - T：telemetry admission/storage/query。
- 当前是高风险共享实现检查点，不是 W2-W 稳定候选。H/R/T在本任务合入前不得修改下列 shared 文件。
- Skiff 尚未发布：直接替换旧 `response.error` 形状，不保留 v1 reader/writer、dual path或按
  code/message升级 fixed。

允许 production 写入：

- `runtime/capability-context/src/lib.rs`仅可 additive re-export现有
  `response::FixedServiceResponseFailure`；不得修改该类型、response模块或F334 diagnostic seam；
- `runtime/request-contract/src/{response_event.rs,lib.rs}`；
- `runtime/transport/Cargo.toml`仅可增加 strict fixed decode所需的现有低层 model依赖；
- `runtime/transport/src/{protocol.rs,response_mapper.rs,lib.rs}`；
- `router/src/protocol/{envelope.ts,runtimeProtocol.ts}`；
- `telemetry/src/protocol.ts`。

允许测试/fixture写入：

- `runtime/transport/src/{protocol/tests.rs,response_mapper/tests.rs}`；
- 新建且只由本任务拥有
  `runtime/transport/testdata/service-error-response-v2*.json`；
- `router/tests/protocol.test.ts`；
- `telemetry/tests/protocol.test.ts`；
- `doc/architecture/fixtures/observability-minimal.json`仅更新已决定的 telemetry parity字段和正负样例。

禁止修改 capability/model error semantics、eval、request/host、Router runtime consumer/gateway、telemetry
server/store/query/redaction、compiler/std及权威设计正文。若 consumer因新 union暂时不能完整 type-check，
在 result列出精确断点交给 H/R/T；不得在 C0 越界迁移 consumer或添加 compatibility。

## response.error v2 固定布局

全局 binary container magic/version及其它 runtime frame继续使用
`skiff-runtime-frame-v1`。只新增
`RESPONSE_ERROR_FRAME_SCHEMA_VERSION = "skiff-runtime-frame-v2"`，并让两种
`response.error`都使用它：

```text
fixed:
  header = {
    schemaVersion: "skiff-runtime-frame-v2",
    type: "response.error",
    requestId: non-empty,
    errorKind: "fixedService"
  }
  payload = exact OpaqueServiceError.encoded_bytes()

control:
  header = {
    schemaVersion: "skiff-runtime-frame-v2",
    type: "response.error",
    requestId: non-empty,
    errorKind: "control",
    error: { code, message, status?, details? }
  }
  payload = empty
```

要求：

1. request-contract新增复用现有 `FixedServiceResponseFailure` 的 typed `ResponseEvent` variant并 re-export；
   不新增第二个 envelope/carrier。
2. Rust header是严格判别 union；Rust encoder/decoder统一验证 exact version/type/kind、非空 requestId、
   variant exact字段集合与 payload presence。control 的 code/message非空，status如存在必须为400–599。
3. fixed payload通过唯一 `OpaqueServiceError::decode` strict验证并保留收到的原始 bytes；mapper encode/decode
   后 byte-equal。不得拆出 `encodedPayload`、stringify或重新编码 envelope。
4. Router TS镜像同一判别 union，并提供一个同时接收 header和 binary payload的 strict decode/validate seam
   供 R消费。它可以返回只读 envelope view，但必须同时保留原 `Uint8Array`，不得成为分类或重编码 owner。
5. TS envelope view严格拒绝 unknown/missing/extra字段、unknown kind/platform identity、空或外围空白的
   owner/key/type/correlation、空或非 byte array的 encoded payload；Internal payload字段也必须 exact。
6. 旧 v1 generic service frame、fixed带 generic error、control缺 generic error、fixed空 payload、
   control非空 payload、malformed JSON和相同 code/message伪装 fixed全部失败关闭。
7. `runtimeProtocol.ts`的 declarative schema、manual validator和 TS interface不再对 nested
   `additionalProperties`互相矛盾；同一 shared corpus同时约束 Rust/TS。字段顺序不是协议，原 payload bytes
   才是 forwarding事实。

## telemetry parity 固定布局

一次原子冻结 Rust transport、Router TS mirror及 telemetry service TS mirror：

- `visibility`：必填有限值 `operational | restricted`；
- `errorId`：可选 top-level非空 string；`restricted` event必须同时有非空 `traceId`和`errorId`；
- Rust `TelemetryEvent`拒绝 unknown top-level字段；TS两侧只接受同一 allowed field/value集合；
- 现有 telemetry protocol常量仍为 `skiff-telemetry-v1`，不引入兼容 reader；
- operational event不得因此获得 stack字段；restricted内容暂仍放现有受限 `error`对象，具体 host投影、
  redaction、storage和 query隔离由 H/T拥有。

更新共享 observability fixture，使现有事件显式为 `operational`，并至少提供带 traceId/errorId的合法
`restricted`样例以及 missing/unknown visibility、restricted缺 correlation的负例。Rust、Router与
telemetry protocol测试必须消费同一事实；本任务不修改 store/query行为。

## Shared corpus与最小探针

`service-error-response-v2*.json`至少覆盖：

- public、Internal、platform三种 fixed有效 frame，以及一个 control有效 frame；
- Rust encode→decode与 TS decode得到相同 kind/identity/traceId/errorId view，payload byte-equal；
- v1、unknown/missing kind、header/envelope extra/missing field；
- fixed/control payload presence互换、fixed带 error、control缺 error；
- malformed JSON、unknown envelope/platform kind、空/带外围空白 identity/correlation、空 payload；
- control空 code/message与非法 status；
- generic control 即使 code/message与 Internal相同也绝不变成 fixed。

验证 owner：

```bash
cargo test -p skiff-runtime-request-contract --lib --no-fail-fast
cargo test -p skiff-runtime-transport --lib service_error_response_v2 -- --list
cargo test -p skiff-runtime-transport --lib service_error_response_v2 --no-fail-fast
cargo test -p skiff-runtime-transport --lib telemetry --no-fail-fast
cargo check -p skiff-runtime-transport --lib
pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
pnpm --filter @skiff/telemetry exec vitest run tests/protocol.test.ts
git diff --check
```

selector必须非零。不得运行完整 Router/telemetry/eval/workspace/root/stable/live。若上述精确 selector名称需要
新增，应让所有新 wire tests共享该前缀。response.error布局、envelope bytes/model validation、telemetry
visibility/errorId shape或 shared corpus变化会使 C0及全部下游 wire证据失效。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f336-error-wire-checkpoint`
- branch：`codex/p5-f336-error-wire-checkpoint`
- 新的一次性开发 Agent；5分钟内先落 request-contract/Rust判别 union与一条 corpus正例，不能先跑完整测试；
- 提交实现及
  `P5-F336-service-error-wire-telemetry-checkpoint-result.md`，返回 commit、Rust/TS/corpus/telemetry
  parity矩阵、临时 consumer断点与自验收证据；
- 不 push，不承接 H/R/T、combined probe或独立验收。
