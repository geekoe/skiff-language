# P5-F445H I7 P8 S3 Deferred response sink propagation result

状态：

```text
PASS
S3_COMPLETE = YES
I_RESUME_UNBLOCKED = YES
PRODUCTION_CHANGE = YES
STANDALONE_ROUTER_BUSINESS_PORT = NO
ROUTER_COVERAGE_COMPOSED_FROM = P8_T
LOWER_SEAM = CONCRETE_HOST_ROUTER_SESSION
```

## 1. Baseline and commits

- 最终integration baseline：
  `f28ecd9a2099c575bfbe6e3aad40296d7157e559`
  （tree `1cd60da6c2c4379914d1abf15e3dd34b45e3bcbb`）
- fixture commit：
  `9308f2549144d0e2f53cd3c42a6bca533c578c81`
  （tree `9d0dceca4a10f2d4890263fa628d14345c2b96f4`）
- production commit：
  `50b3c580010d669ea79d8f1e1cf753f5beaa7bc3`
  （tree `061471beb40783079f0019cd1baf759b2dcf9a5a`）
- branch：
  `codex/p5-f445h-i7-p8-s3-response-sink`
- integration owner：
  `/root/phase05_integration_steward`

该baseline包含S2和S3 evidence-rule correction。fixture checkpoint在修复前稳定RED两次；其后才进入
production实现。

## 2. RED classification and identity trace

临时trace证明三个stream始终属于同一deferred producer registry，请求generation均为`1`：

| role | stream | create/register/lookup |
| --- | --- | --- |
| raw HTTP entry outer producer | `stream-0` | identity一致 |
| overlay-local argument producer | `stream-1` | identity一致 |
| dependency `wrapWithResponseSink` producer | `stream-2` | identity一致 |

首次偏离不是stream id、registry或generation，而是response-sink env handoff：

```text
runtime_http_gateway serverStream
  -> execute_runtime_assembly_addr_with_stream_defer(..., Env::new())
  -> stream-0 parked producer已有自己的stream sink
  -> producer env中的response sink仍为absent

dependency wrapWithResponseSink native call
  -> current stream sink present，identity = stream-2 sink
  -> response sink absent，identity = none
  -> std.http.emitResponseStream进入response-context failure
```

`program_invocation`中写入response sink的路径没有经过。native失败后的cancel/cleanup最终显示
`unknown Stream value`，但它发生在response-context failure之后，是secondary cleanup error，不是primary
stream transport偏离。所有临时trace和环境开关均已撤回。

## 3. Minimal repair

修复严格限于两个冻结production owner：

```text
runtime/eval/src/program_stream.rs
runtime/eval/src/runtime_http_gateway.rs
```

`runtime_http_gateway`在得到handler返回的canonical stream value后、启动
`drive_deferred_stream_producer`之前：

1. 从返回值取得exact `stream-0` id；
2. 在既有deferred producer registry中定位同一个parked producer；
3. 核对完整stream value和request generation；
4. 把该producer已经拥有的stream sink以既有`TypedStreamSink` view附着到其producer env；
5. 沿原drive路径运行producer。

嵌套调用继承该env后，`wrapWithResponseSink`中的native response sink指向`stream-0`，而current stream
sink仍是`stream-2`；二者职责和identity没有合并。missing id、entry已被take、stream/request mismatch和
重复附着都返回错误，保持fail closed。

变更边界：

| 项目 | 是否新增 |
| --- | --- |
| sink owner | NO |
| channel | NO |
| registry | NO |
| global/request state | NO |
| public API | NO |

没有修改`program_execution`、`program_invocation`、`env`、Router、protocol、schema、compiler、
test-runner production、std或Internals。

## 4. GREEN matrix

同一个compiled、linked、admitted `kind:test` fixture经
`RuntimeHost::dispatch_router_binary_frame`和实际`RouterWriterMessage` sink运行三个case：

| case | Host frames / terminal | cleanup |
| --- | --- | --- |
| normal | `start(200)`、一个`chunk("body")`、一个`end`，无第二terminal | request supervisor active=`0` |
| producer error | `start(200)`、保留native `chunk("before-error")`、一个error terminal，无第二terminal | request supervisor active=`0` |
| runtime cancel | `start(200)`、`chunk("first")`后发送真实`request.cancel`，无晚到frame | request supervisor active=`0` |

三个case共用S2已证明的`stream-0/1/2`生命周期和同一request scope。response sink只是`stream-0`既有sink的
typed view，不是第二owner；request关闭、唯一terminal或cancel完成后producer env被释放。最终GREEN中不再
出现native response-context failure，Host terminal也不再被`unknown Stream value`遮蔽。

该实验只证明runtime `request.cancel`，不声称external socket/client disconnect，也没有启动standalone
Router或business port。

## 5. Outside-context negative and regressions

临时validation-only单元probe直接构造没有response sink的
`HttpResponseStreamCapabilityContext`，调用`response_item_type`，稳定得到：

```text
std.http.emitResponseStream used outside a raw HTTP streaming response context
```

probe结果为`1 passed; 0 failed`，随后已完整撤回，最终写集不含测试专用context。修复只在raw HTTP gateway
对exact parked producer附着sink，普通调用无法获得response sink。

回归结果：

| selector | 结果 |
| --- | --- |
| `deferred_package_direct_stream_keeps_raw_http_response_sink` | PASS；最终复验`1 passed; 0 failed`，内部三个case |
| `package_direct_stream_producer_argument_real_gateway`（S2） | PASS；`1 passed; 0 failed` |
| `package_direct_http_stream_registry_return_stream_reaches_real_gateway`（S1） | PASS；`1 passed; 0 failed` |
| native response emit prepared-wait回归 | PASS；`1 passed; 0 failed` |

selector中的`real_gateway`是历史名称，只表示concrete Host gateway/session lower seam，不表示standalone
Router。

## 6. Validation

执行：

```text
cargo test --locked -p skiff-runtime-host \
  deferred_package_direct_stream_keeps_raw_http_response_sink -- --nocapture
```

修复后连续两次均为`1 passed; 0 failed`，格式整理后的最终复验也为`1 passed; 0 failed`。

其它检查：

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p skiff-runtime-host package_direct_stream_producer_argument_real_gateway -- --nocapture` | PASS |
| `cargo test --locked -p skiff-runtime-host package_direct_http_stream_registry_return_stream_reaches_real_gateway -- --nocapture` | PASS |
| `cargo test --locked -p skiff-runtime-eval emit_response_stream -- --nocapture` | 命令成功但命中`0`个测试，未伪报为行为证据 |
| `cargo test --locked -p runtime emit_response_stream -- --nocapture` | baseline compile blocker；根runtime旧测试仍使用已变化的DB/type API，测试未运行 |
| 临时capability-context outside-context probe | PASS，`1 passed; 0 failed`，已撤回 |
| `cargo test --locked -p skiff-runtime-native dispatch::prepared_tests::http::http_stream_sse_and_response_emit_prepare_owned_waits_without_starting_them -- --nocapture` | PASS |
| `cargo check --locked -p skiff-runtime-eval -p skiff-runtime-host` | PASS |
| `rustfmt --edition 2021 --check <本任务Rust写入文件>` | PASS |
| `git diff --check` | PASS |
| `cargo fmt --all -- --check` | baseline-known FAIL，仅剩本任务未修改的`compiler/tests/package_imports.rs`三处格式差异 |

没有为根`runtime`或全仓格式基线阻塞扩大S3写集。未运行I、完整AIHub、J生态gate、
stable/live/network/Mongo/OAuth/browser。

## 7. Actual write set and handoff

```text
runtime/eval/src/program_stream.rs
runtime/eval/src/runtime_http_gateway.rs
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs
runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs
test-runner/fixtures/package-direct-http-stream-registry/argument-provider/**
test-runner/fixtures/package-direct-http-stream-registry/argument-tests/**
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-S3-deferred-response-sink-propagation-result.md
```

结论：

```text
S3_COMPLETE = YES
I_RESUME_UNBLOCKED = YES
```

由`/root/phase05_integration_steward`集成、清理并恢复I；本任务不自行merge、push或启动I。
