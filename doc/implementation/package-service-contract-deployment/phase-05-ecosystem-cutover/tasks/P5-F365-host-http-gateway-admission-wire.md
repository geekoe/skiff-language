# P5-F365 Host HTTP gateway admission and wire execution

状态：Ready（C3 Host leaf；依赖 F363 exact request/eval seam，与后续test-runner真实调度解耦）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F358-runtime-assembly-http-gateway-linking-result.md`
- `P5-F359-http-gateway-request-protocol-result.md`
- `P5-F363-runtime-http-gateway-execution-seam-result.md`

父节点已经冻结 linked gateway facts、HTTP-only request wire、typed/raw/stream execution seam 与
Router/Host timeout职责。本任务只把Host active assembly、wire admission、capability context和response
sender接到该seam；不修改共享DTO或重新解释adapter。

## Exact base

- integration commit：`ac98e3057ca5a16434d92430b0356e3451d91ab3`
- integration tree：`61273a77972004f2827443ef66c0fe86eaa846f5`
- branch：`codex/package-service-phase-05`

当前Host仍引用已删除的`GlobalIngressBinding`和canonical wire旧WebSocket/operation字段，31个编译错误
集中在`assembly_admission.rs`、`assembly_wire.rs`及旧WebSocket request bridge。F363已经提供
`RuntimeAssemblyHttpGatewayTarget`、`execute_runtime_http_gateway_request`和最小Host eval adapter trait。

## 必须完成

1. `ActiveAssemblyRoute`改为HTTP gateway route：
   - pin active assembly/generation；
   - pin selector命中的同一个`Arc<LinkedGatewayEntry>`；
   - activation只由entry exact owner deployment取得；
   - route暴露exact key/identity/protocol surface、activation、execution image与context set；
   - 不保存或查找ServiceContract、BoundaryOperationDescriptor、provider operation target。
2. active assembly admission/validation按F358 candidate检查：
   - assembly `gatewayIngress` selector集合与candidate ingress exact一致；
   - selector lookup与`(owner,key)` lookup必须是同一`Arc`；
   - binding deployment/key/identity与linked entry逐值一致；
   - owner activation与deployment record存在；
   - 当前只允许HTTP，typedJson只允许unary，rawHttp允许unary/serverStream；
   - internal service operation store/context仍保留，不能误删。
3. `assembly_wire`只接受F359 canonical HTTP request：
   - selector、assembly identity/generation、nested gateway identity、mode与linked entry逐值一致；
   - `httpRequest` URL/host/method/path与routing一致，body bytes保持opaque；
   - caller只能是gateway；
   - 不接受/构造contract operation、top-level gateway identity、handler/adapter/schema、
     testEffectDoubles或RuntimeAssembly WebSocket metadata。
4. wire admission后只构造`RuntimeAssemblyHttpGatewayTarget`并调用F363 seam。Host eval adapter仅建立
   activation-owned config/db/file/http/outbound/actor/telemetry等capability context，不解析gateway
   callable、signature、schema或adapter args。
   - gateway handler中的internal service call继续经过service boundary；
   - activation-owned service/build/protocol facts可供Actor/DB等本地capability使用，但不得写回
     external request wire或伪造service caller。
5. Host deadline必须clamp并执行同一deployment policy：

   ```text
   min(
     wire timeoutMs deadline when present,
     wire expiresAt remaining deadline when present,
     exact deployment policy.timeoutMs when present
   )
   ```

   - malformed expiresAt fail closed；
   - already-expired或zero deadline立即超时；
   - absence不能覆盖deployment policy；
   - Host不能延长Router传入的deadline；
   - execution budget、outbound HTTP timeout与telemetry观察同一effective deadline。
6. unary response继续使用既有response mapper与HTTP response ceiling。serverStream通过F363
   `ResponseEventSink`逐帧映射为现有response.start/chunk/end transport frame；错误、取消、超时、
   sender关闭与stream cleanup必须只有一个terminal owner。
7. canonical RuntimeAssembly WebSocket request bridge从HTTP路径删除并明确fail closed；不得恢复F359
   已删除的WebSocket header、建立local compatibility DTO或设计业务消息协议。Host通用
   WebSocket generation lifecycle registry/ack/release/disconnect owner必须保留，不因本任务误删。
8. 更新Host直接tests/fixtures，至少覆盖：
   - typed unary、raw unary、raw server-stream真实wire → private handler → response frames；
   - exact selector/key/identity/generation/mode及错值负例；
   - 非UTF-8 body保持opaque；
   - deployment timeout clamp、wire更短、expiresAt更短、expired/malformed；
   - response ceiling、cancel、reload route pin、stream single terminal；
   - internal service operation与通用WebSocket generation lifecycle回归。

## 写入范围

主要owner：

- `runtime/host/**`；
- 仅为typed deadline/supervisor接入所必需的`runtime/request`局部helper/tests；
- 直接Host fixtures/tests。

禁止：

- artifact/deployment/identity/compiler；
- `runtime/loader/**`、`runtime/linker/**`、`runtime/eval/**`、`runtime/transport/**` canonical DTO；
- Router、test-runner、三仓库service、stable/live配置。

`Cargo.lock`仅在Cargo根据实际production dependency机械要求时更新；不得出现无关版本变化。若正确Host
接入要求修改F358/F359/F363公共事实，立即返回`TASK_SCOPE_EXPANDED`。

## 验证

先枚举Host gateway selector并确认非零，再运行：

```bash
cargo test -p skiff-runtime-host <http-gateway-selector> -- --list
cargo test -p skiff-runtime-host <http-gateway-selector>
cargo test -p skiff-runtime-host websocket_generation -- --list
cargo test -p skiff-runtime-host websocket_generation
cargo check -p skiff-runtime-host -p skiff-runtime-request -p skiff-runtime-eval
rustfmt --edition 2021 --check <changed-rust-files>
git diff --check
```

production HTTP route/wire反向搜索不得包含
`GlobalIngressBinding|global_ingress|ContractOperationId|contract_operation_id|ServiceContractRef|
websocket_adapter|websocketEntryId|test_effect_doubles`。不运行workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f365-host-http-gateway`
- branch：`codex/p5-f365-host-http-gateway`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入`P5-F365-host-http-gateway-admission-wire-result.md`；
- worktree保持clean，不merge/rebase integration，不push。
