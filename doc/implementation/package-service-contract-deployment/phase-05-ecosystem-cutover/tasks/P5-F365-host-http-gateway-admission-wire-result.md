# P5-F365 Host HTTP gateway admission and wire execution result

状态：Completed（C3 Host HTTP canonical gateway leaf；RuntimeAssembly WebSocket业务入口继续
fail closed，通用WebSocket generation lifecycle保留）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `ac98e3057ca5a16434d92430b0356e3451d91ab3` | `61273a77972004f2827443ef66c0fe86eaa846f5` |
| task checkout | `040c25ea43110a5149b09ceec8edfbad4bf339c7` | `af72abc5f0b8f4730f2871a82584aa09ef5c761d` |
| incoming dirty-state checkpoint | `9b810364deb333bbced956f711acf2d998427365` | `15a7cc84d1838ddb23c1070a0efff0e8ce5105e6` |
| F370 source | `7ddcbf31b06bdc86212b0f406df55f49c06f1231` | `0d46a1df6811f34da8586a6dac25c1715e0165d5` |
| F370 integrated cherry-pick | `44a55f6a2c20e72e8695fe2352319005a4637a21` | `16736eabd36e53829fe48cf6990e60fee381d53a` |
| completed production/tests | `269bda5081bc437a37a4d142e8be00ef5ceaa5ea` | `cb6a67dc9554636dd2a9b2e6e689e5af0f6ad70e` |

工作分支为`codex/p5-f365-host-http-gateway`，worktree为
`/Users/geek/workspace/skiff-p5-f365-host-http-gateway`。resume开始时先把20个tracked path的既有
dirty diff原样保存为checkpoint，再以普通cherry-pick接入F370；没有merge/rebase integration或
main，没有修改F358/F359/F363共享DTO/identity、Router、test-runner production、stable/live配置，
也没有push。

## 2. Active route与whole-assembly admission

- `ActiveAssemblyRoute`固定同一个`Arc<ActiveAssembly>` generation、selector命中的
  `Arc<LinkedGatewayEntry>`、entry owner的`ActivationContext`与deployment policy。
  `request_target()`只由该entry与activation-owned execution image/context构造
  `RuntimeAssemblyHttpGatewayTarget`，不保存或恢复ServiceContract、operation descriptor或provider
  operation target。
- route lookup同时按selector与`(owner, gatewayEntryKey)`查找，并要求两者`Arc::ptr_eq`；因此一个
  request不能把selector facts与另一个linked entry拼接。
- `validate_candidate`逐值核验assembly `gatewayIngress`集合、owner/key/identity、deployment entry
  protocol surface、deployment ingress binding、activation存在性与HTTP adapter/mode：
  `typedJson + unary`、`rawHttp + unary/serverStream`是唯一允许组合。
- internal service operation的contract store、operation descriptor与activation contexts继续保留；
  额外25项admission/execution/recovery回归以及ordinary internal service operation均通过。

## 3. Canonical HTTP wire与F363 execution seam

- Host只接受F359 typed `RuntimeAssemblyRequestStartFrameHeader`：
  - caller精确为gateway，ingress精确为HTTP；
  - selector、assembly identity/generation、nested gateway identity、dispatch mode与active route逐值
    一致；
  - `httpRequest.method/path`与routing一致，URL要求HTTP scheme、无credentials/hash、exact
    host/path；
  - binary payload按`Vec<u8>`原样交给F363 seam，非UTF-8 body不经过字符串或JSON重解释。
- admission后唯一execution target是`RuntimeAssemblyHttpGatewayTarget`；Host eval adapter只组装
  activation-owned config、DB、file、HTTP/outbound、Actor、telemetry与request budget contexts，不
  解释gateway callable、signature、schema或adapter args。
- F370修正request-local generation与assembly generation混淆：校验现在读取activation identity中的
  assembly generation。相同pinned assembly上的连续request generation回归为8项request-layer
  selector的一部分并通过。
- canonical HTTP路径删除旧RuntimeAssembly WebSocket request bridge；eval capability adapter不再
  从request extra读取`websocketEntryId`，WebSocket capability明确以`None`构造。

## 4. Deadline、response ceiling与single terminal

- Host effective deadline为：

  ```text
  min(
    wire timeoutMs,
    wire expiresAt remaining time,
    exact deployment policy.timeoutMs
  )
  ```

  malformed RFC3339直接拒绝；expired或zero deadline立即超时；缺失wire deadline不能覆盖deployment
  policy。clamped deadline写回typed header，supervisor execution budget、Host timer、outbound
  capability context与telemetry观察同一个值，Host不会延长Router deadline。
- unary继续经过既有response mapper与HTTP response ceiling。server stream由
  `HostHttpGatewayResponseSink`逐帧映射start/chunk/end并累计byte ceiling。
- stream ceiling溢出时sink先原子标记terminal，再发送canonical
  `ResourceLimitExceeded` error并把同一错误返回给F363 cleanup；后续finish/cancel不能发送第二个
  terminal。sender关闭、取消、超时与正常end也共享该terminal state。
- route一直持有到execution、取消、response映射和supervision全部结束，reload期间in-flight request
  继续使用旧route；stale wire generation在新请求admission时fail closed。

## 5. WebSocket containment与fixture closure

- canonical request-entry WebSocket bridge文件已删除，但Host通用generation
  acquire/ack/release/disconnect registry继续存在；reload前route在disconnect前保持pinned，release
  exact/idempotent与rejection rollback三项回归全部通过。
- direct Host package fixtures更新为canonical `pkg-callable:<package>:<public-path>` identity，并补齐
  implementation function links及generic callable type-parameter scope。这只修复test fixture的当前
  artifact identity事实，没有扩展到artifact/compiler/shared owner。
- `Cargo.lock`只机械增加Host direct fixture所需的`skiff-compiler`与`skiff-test-runner`
  dev-dependency edges，没有版本变化。

## 6. Verification

所有指定selector先枚举并确认非零：

| selector | enumerated | execution |
| --- | ---: | ---: |
| `skiff-runtime-host host_http_gateway` | 6 | PASS；6/6 |
| `skiff-runtime-host websocket_generation` | 3 | PASS；3/3 |
| `skiff-runtime-request runtime_http_gateway` | 8 | PASS；8/8 |
| `skiff-runtime-host loader::assembly_admission::tests::` | 25 | PASS；25/25 |
| `skiff-runtime-eval ingress_hands_fixed_failure_up_without_importing_an_external_caller` | 1 | PASS；1/1 |

| 命令 / gate | 结果 |
| --- | --- |
| `cargo check -p skiff-runtime-host -p skiff-runtime-request -p skiff-runtime-eval` | PASS；只有既有dead-code/unused warnings |
| `rustfmt --edition 2021 --check <changed-rust-files>` | PASS |
| production HTTP route/wire反向搜索 | PASS；零匹配 |
| `git diff --check` | PASS |

反向搜索集合为：

```text
GlobalIngressBinding|global_ingress|ContractOperationId|contract_operation_id|
ServiceContractRef|websocket_adapter|websocketEntryId|test_effect_doubles
```

搜索范围包括Host canonical request execution/wire、request-layer HTTP target/execution，以及
`ActiveAssemblyRoute`、route lookup和HTTP ingress candidate-validation owned spans，结果均为零。
`ActiveAssembly`的internal service contract store API刻意保留在该HTTP route范围之外。

未运行workspace/root gate、stable/live instance或跨仓库验证。

## 7. 自验收矩阵

| 任务条款 | 代码证据 | 测试/反向证据 |
| --- | --- | --- |
| exact active route pin | active generation + exact linked entry `Arc` + owner activation/context | selector/owner-key同Arc、reload old-route pin、stale generation负例 |
| candidate admission | ingress集合、binding facts、activation、protocol/mode逐值核验 | admission/execution/recovery 25/25 |
| canonical opaque HTTP wire | typed header exact validation；body保持bytes | typed/raw/stream、非UTF-8、wrong assembly/generation/identity/mode/method/path/URL |
| F363 minimal seam | `RuntimeAssemblyHttpGatewayTarget` + activation-owned eval adapter | request seam 8/8；internal service operation与fixed service failure回归 |
| effective deadline | wire timeout/expiry/deployment policy唯一min并写回budget | policy、wire更短、expiry更短、expired、malformed |
| ceiling/single terminal | unary ceiling + stateful stream sink terminal owner | unary/stream oversize、cancel、sender/cleanup、无第二帧 |
| WebSocket fail closed/containment | canonical bridge删除；generic lifecycle registry保留 | legacy bridge拒绝；generic generation 3/3；owned反搜零 |
