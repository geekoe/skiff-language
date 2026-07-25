# P5-F340 Service error request/host/session consumer

状态：Ready。

## 直接父节点

- 当前 production 跳点、H owner、W1/S1 probes：
  `P5-F333-wire-observability-delta-audit-result.md`
- 已冻结的 typed restricted diagnostic handoff：
  `P5-F335-restricted-service-diagnostic-acceptance-result.md`
- 已冻结并通过复验的 shared wire/telemetry checkpoint：
  `P5-F339-response-error-schema-reacceptance-result.md`

父节点已沿引用链连接唯一权威设计。本任务只实现 H：request/host/session consumer；不改 fixed
semantic owner、shared frame/telemetry DTO、Router或 telemetry service。

## 起点、目标与硬约束

- 起点 commit：`e3095ec642d49b59955f5f48a2950eafc9d92571`
- 起点 tree：`6b7fce6db07d7fde3b88609539150c53f5608e62`
- 任意 fixed service failure必须从
  `EvalRuntimeError::FixedServiceFailure(OpaqueServiceError)`以 typed方式提取，进入
  `ResponseEvent::FixedServiceFailure`；不得调用`WirePayload::payload()`、`RequestError::response_error()`
  或按 code/message/details分类。
- exact `OpaqueServiceError.encoded_bytes()`从 eval→request→host→Rust frame以及反向 session decode
  必须 byte-for-byte不变。
- generic control/pre-ingress错误仍走`ResponseEvent::Error(ResponseError)`，不得按恰好相同的
  code/message升级为 fixed。
- host必须把 Router ingress已有 trace/span应用到 assembly
  `RequestTelemetryContext`，不得生成第二个 trace。
- fixed operational event只含有限 kind/correlation和预算等安全信息；top-level traceId/errorId直接取
  fixed envelope。不得把 fixed送入`response_error_to_telemetry_map`。
- F335 typed sink必须接入真实 host telemetry：
  - 每个 provider export hop发一条`visibility=restricted`事件；
  - owner使用 diagnostic 自带的 provider service/activation/operation；
  - correlation使用 diagnostic 自带 traceId/errorId；
  - source与当前 provider的完整 local stack投影为有界结构；
  - 不携带 error payload/display、heap/runtime value/type address；
  - sink失败不得改变、替换或阻断原 fixed service response。
- 所有通过 host `telemetry_event`构造的普通事件显式为`operational`；新增 top-level errorId正确初始化。
- restricted stack可包含 typed source span或有限 synthetic reason，以及脱敏 remote-boundary frame；
  不得引入源码路径、函数名或原始私有值。producer现有大小限制与 secret-key redaction必须覆盖新增结构。

## Production 写入边界

允许修改：

- `runtime/request/src/{error.rs,assembly_ingress.rs,runner.rs,lib.rs}`中的最小 typed extraction、
  re-export与 legacy telemetry helper隔离；
- `runtime/eval/src/error.rs`仅可增加供 request跨 crate调用的严格 fixed carrier提取 API；
  不得修改 error variant、display、conversion、service envelope或异常语义；
- `runtime/host/src/telemetry.rs`；
- `runtime/host/src/capability_context/{telemetry.rs,effect_context.rs,native_projection.rs}`中接入 typed
  restricted sink所需的最小相邻实现；
- `runtime/host/src/eval_capability_adapter/{effects.rs,http.rs,request_contexts.rs,mod.rs}`中把同一 request
  telemetry/sink接到 eval capability contract；
- `runtime/host/src/host/{request_entry.rs,request_entry/assembly.rs,request_entry/assembly_wire.rs,
  request_supervisor.rs,request_trace.rs,router_session.rs,http_response_ceiling.rs,telemetry.rs}`。

上述文件只有与本任务直接相关的最小修改。若确认还需 host/request crate中的一个相邻 production文件，先在
result精确说明原因；不得越到 capability-context/eval service channel、request-contract、transport或其它
crate。

明确禁止修改：

- `runtime/model/src/service_error.rs`；
- `runtime/capability-context/**`；
- `runtime/eval/src/assembly_execution/**`；
- `runtime/request-contract/**`；
- `runtime/transport/**`；
- `router/**`、`telemetry/**`；
- shared corpus、权威设计、父 task/result、lockfile。

## 必须完成的路径

1. assembly unary成功/普通错误/fixed错误三分支完整；HTTP response ceiling把 fixed视为合法终止错误，不
   当作 response body。
2. supervisor为 generic control保留现有安全 operational error逻辑；为 fixed新增独立 typed completion，
   top-level errorId/traceId正确且无 full stack/private payload。
3. assembly trace fields从 request extra进入 start/end/error与 restricted event。
4.真实 eval capability context安装 F335 restricted sink；clone后仍指向同一 request telemetry emitter，
   default discard不再是 production assembly请求的实际路径。
5. `router_session`只对`response.error`调用 C0 dedicated decoder；fixed payload允许且必须非空，
   control payload必须为空；`response_error_to_outbound(header, payload)`返回 typed carrier。
6. public/Internal/platform三种 fixed bytes均保持；相同 code/message的 generic control仍 generic。
7. cancel、success、package-local错误与非 assembly control-plane telemetry不被错误标记为 restricted。

## 测试与验证

测试允许写入：

- `runtime/request/src/**`现有 co-located tests；
- `runtime/host/src/**`现有 co-located tests与
  `runtime/host/tests/**`中直接覆盖本任务路径的 focused fixture；
- 不新建跨 Router/telemetry service 的 C1 测试。

至少证明：

- request typed extraction绕过 generic payload；
- assembly fixed producer三种 envelope exact bytes；
- generic同 code/message不升级；
- Router session fixed/control正反例与 exact bytes；
- ingress trace传播；
- operational fixed event无 stack/private sentinel且带同一 trace/errorId；
- restricted sink真实接线、每 hop一条、本地 stack结构保留、secret值被遮罩、sink failure不改响应；
- TelemetryEvent构造全部有 visibility/errorId初始化。

先列出非零 selector，再运行：

```bash
cargo test -p skiff-runtime-request
cargo test -p skiff-runtime-host
cargo check -p skiff-runtime-host
git diff --check
```

若完整 host test因本阶段另一个未合流 consumer产生非本任务断点，必须给出精确证据并运行覆盖改动模块的非零
focused selectors。不得运行完整 workspace/root、stable/live，不 push。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f340-service-error-host`
- branch：`codex/p5-f340-service-error-host`
- 新的一次性开发 Agent；
- 新增`P5-F340-service-error-host-consumer-result.md`，写明 typed路径、telemetry投影、exact-byte证据、
  selector/数量、所有越界必要性与剩余 blocker；
- 提交并返回 implementation commit，不修改 task 状态，不承接后续验收。
