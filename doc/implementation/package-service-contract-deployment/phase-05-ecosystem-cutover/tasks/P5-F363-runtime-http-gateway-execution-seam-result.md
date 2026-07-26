# P5-F363 Runtime HTTP gateway execution seam result

状态：Completed（C3 shared Rust request/eval leaf；Host admission/wire、Router、transport、
loader/linker与artifact事实未修改）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `b71e622ca35109519e904f269a67f19bc2f08de4` | `7d79c140534db0ed2336e3babe511fa444fdc2e6` |
| task checkpoint | `dff07b75dadabb5ebd624302634239ce728bf547` | `6e98e67e227004a1f3c63c4afb0e1005006b2cb9` |
| production/tests | `d7ad2694576ad5bb4eba120dd7d3abf6b29204ee` | `6427a2c2226d7255fde5adce01654195f14c4a2c` |

工作分支为`codex/p5-f363-runtime-http-gateway-seam`，worktree为
`/Users/geek/workspace/skiff-p5-f363-runtime-http-gateway-seam`。本leaf没有merge/rebase
integration，没有运行workspace/root、Host或stable/live，没有修改transport、loader、linker、
artifact/deployment/compiler、Router、test-runner、lockfile或三仓库service源码，也没有push。

## 2. Exact request-owned target

- `RuntimeAssemblyHttpGatewayTarget`与现有service-call target并列；它只持有同一个
  `RuntimeAssemblyEvalTarget`、F358 `LinkedGatewayEntry`及handler/pre/guard的exact
  `ExecutableAddr`，没有构造service caller、contract operation或boundary operation descriptor。
- 构造时逐项验证entry owner等于request activation deployment，eval activation与
  request receiver为同一个`Arc`，execution image ready且包含implementation package。
- `GatewayEntryKey`和`GatewayEntryIdentity`重新strict parse；identity从protocol surface重新计算，
  surface、adapter args、dispatch mode、external sources、handler parameter coverage与pre/guard形状
  均fail closed。
- 每个callable只接受implementation package内唯一且逐值相等的
  `PackageLocalAbiSymbol::Callable` signature和`InternalFunction` target；execution image中的
  executable地址、kind、self、return、type params、suspend flag、parameter count/name也必须一致。
  没有display/source/public symbol、短名或service-operation fallback。
- valid-but-wrong request payload key在Eval入口与target key逐值比较后拒绝；F358 candidate仍是
  `(owner, key)`与selector到同一`Arc<LinkedGatewayEntry>`的唯一来源。F359 wire不新增key或任何
  legacy selector字段。

## 3. Request/Eval execution seam

- `execute_runtime_http_gateway_request`直接消费F359 typed
  `RuntimeAssemblyRequestStartFrameHeader`与opaque body bytes，并验证canonical schema/type/caller/
  routing、assembly identity/generation、gateway identity、HTTP method/path、mode与linked surface/plan。
- 最小Host API只要求admitted linked target、header/body、activation-owned execution handles及
  capability-context adapter。`RuntimeHttpGatewayEvalAdapter`只接收eval pin和request metadata，
  Host无需解释adapter args、schema或callable signature。
- `typedJson` unary先执行可选guard；guard short-circuit不会触碰body。之后执行可选pre，并只在
  exact handler arg adaptation处用既有typed JSON/type-plan codec解码opaque body；handler结果再由
  同一codec编码为JSON response。
- `rawHttp` unary用既有binary HTTP boundary构造exact `std.http.HttpRequest`并解码
  `std.http.HttpResponse`，body bytes不经JSON或字符串重解释。
- `rawHttp` server stream要求exact
  `Stream<std.http.HttpResponseStreamEvent>`，只投递start/chunk/end。`typedJson + serverStream`、
  mode/surface/plan错配、错误target/signature及非法terminal序列全部fail closed。
- request lifecycle无论成功或失败都结束request activation；cancellation flag会先cancel activation。
  test-effect boolean仅选择既有internal fixture runtime，不在canonical wire中引入
  `testEffectDoubles` sequence。

## 4. Shared stream and terminal ownership

- gateway stream调用新增的exact assembly callable入口，但继续使用既有deferred producer
  scheduler、`drive_deferred_stream_producer`、cancel signal和cleanup guard。
- `materialize_runtime_stream_item`从既有`for in`路径提取同一套跨heap carrier deep-clone逻辑；
  gateway的in-process stream允许internal item，既有external HTTP stream consumer仍保持
  `allow_internal_items = false`并继续执行wire decode/type-plan检查，外部trust boundary语义未放宽。
- 每个item仍通过linked item plan与既有HTTP event codec；未新增stream scheduler、payload framing、
  HTTP type layout或JSON codec。
- `ResponseStreamWriter`继续是single-terminal owner，并新增完成态校验：
  仅`start/chunk*/end`有效，缺失、乱序及重复terminal拒绝。response ceiling仍由既有
  `ResponseEventSink`/Host downstream owner实施，本seam没有建立第二套ceiling。

## 5. Direct test evidence

Eval fixture真实运行compiler authoring、canonical std seed、deployment assembly resolution与linker
execution image；handler/pre/guard都是implementation package的private callable。

| 测试 | 证据 |
| --- | --- |
| `runtime_http_gateway_typed_unary_runs_exact_guard_pre_and_private_handler` | typed body、guard、pre context与private handler exact执行 |
| `runtime_http_gateway_guard_short_circuits_before_typed_body_decode_and_pre` | guard返回204；非法UTF-8 body未被提前解码 |
| `runtime_http_gateway_raw_unary_preserves_binary_http_context_and_body` | raw request和任意binary body逐字节返回 |
| `runtime_http_gateway_raw_server_stream_uses_exact_start_chunk_end_sequence` | real private stream handler产生唯一start/chunk/end |
| `runtime_http_gateway_stream_cancellation_cleans_up_and_next_stream_completes` | callback cancellation清理producer；下一stream正常完成 |
| `runtime_http_gateway_wrong_target_signature_mode_and_adapter_fail_closed` | valid wrong key、wrong executable、signature、mode和adapter kind拒绝 |
| request target fact tests | noncanonical key、wrong identity/surface/plan/signature和wrong owner拒绝 |
| response writer tests | exact terminal接受；missing/out-of-order/repeated terminal拒绝 |

为使当前integration checkpoint中的eval direct fixtures继续编译，聚焦fixtures机械迁移
`global_ingress`到F358 `gateway_ingress`，并补齐F354已required的`service_call_roots`。没有改变这些
fixtures的执行语义。

## 6. Service-operation separation and reverse search

既有internal service operation回归
`ingress_hands_fixed_failure_up_without_importing_an_external_caller`继续通过；gateway target没有替代
service-call入口。既有stream cancellation回归
`provider_stream_consumer_cancel_is_control_terminal`也继续通过。

production seam执行：

```text
rg -n \
  'ContractOperationId|ServiceContractRef|RuntimeAssemblyServiceCallTarget|contract_operation_id' \
  runtime/request/src/http_gateway_target.rs \
  runtime/request/src/http_gateway_execution.rs \
  runtime/eval/src/runtime_http_gateway.rs

rg -n \
  'httpAdapter|testEffectDoubles|websocket|WebSocket' \
  runtime/request/src/http_gateway_target.rs \
  runtime/request/src/http_gateway_execution.rs \
  runtime/eval/src/runtime_http_gateway.rs
```

两组结果均为零匹配。

## 7. Verification

Selector先枚举并确认非零：

| selector | 枚举结果 |
| --- | --- |
| `skiff-runtime-eval runtime_http_gateway` | 6 tests |
| `skiff-runtime-request runtime_http_gateway` | 4 tests |
| internal service-operation regression | 1 test |
| shared stream cancellation regression | 1 test |

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval runtime_http_gateway -- --list` | PASS；6 tests，非零 |
| `cargo test -p skiff-runtime-eval runtime_http_gateway -- --test-threads=1` | PASS；6/6 |
| `cargo test -p skiff-runtime-request runtime_http_gateway -- --list` | PASS；4 tests，非零 |
| `cargo test -p skiff-runtime-request runtime_http_gateway` | PASS；4/4 |
| `cargo test -p skiff-runtime-eval ingress_hands_fixed_failure_up_without_importing_an_external_caller -- --list`及执行 | PASS；1/1 |
| `cargo test -p skiff-runtime-eval provider_stream_consumer_cancel_is_control_terminal -- --list`及执行 | PASS；1/1 |
| `cargo check -p skiff-runtime-eval -p skiff-runtime-request` | PASS；仅dependency既有unused/dead-code warnings |
| `rustfmt --edition 2021 --check <all changed Rust files>` | PASS |
| `git diff --check` | PASS |
| `git diff --exit-code -- Cargo.lock` | PASS；零差异 |

## 8. 自验收矩阵

| 任务条款 | production/test证据 | 结果 |
| --- | --- | --- |
| exact gateway target | eval/activation/owner/key/identity/surface/plan/callable/signature/image逐项pin | PASS |
| 与service-call并列 | production旧operation类型反搜零匹配；internal operation回归通过 | PASS |
| typed/raw/stream矩阵 | real compiler/linker fixture的typed unary、raw unary、raw stream 3类正例 | PASS |
| guard/pre/private handler exact | guard short-circuit、pre context及不同executable address断言 | PASS |
| fail closed | owner/key/identity/target/signature/mode/adapter/terminal负例 | PASS |
| shared codecs/heap/stream/cancel | 既有boundary/type-plan、deferred scheduler、heap transfer与cleanup复用 | PASS |
| opaque binary HTTP wire | body只在Eval arg adaptation解码；legacy/WebSocket字段反搜零匹配 | PASS |
| minimal Host seam | Host adapter只构造capabilities，不解释gateway facts | PASS |
| ownership与运行边界 | diff仅request/eval、本result与最小Cargo依赖；lockfile/禁止域零修改 | PASS |

本任务不需要修改F358 linked公共事实、F359 wire或Host，因此未触发`TASK_SCOPE_EXPANDED`。
