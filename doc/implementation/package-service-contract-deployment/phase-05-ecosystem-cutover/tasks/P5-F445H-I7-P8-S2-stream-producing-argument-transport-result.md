# P5-F445H I7 P8 S2 Stream-producing argument transport result

状态：

```text
PASS
S2_COMPLETE = YES
S3_UNBLOCKED = YES
I_RESUME_UNBLOCKED = NO
PRODUCTION_CHANGE = NO
RED_ONLY_COMPARISON_RUN = NO
STANDALONE_ROUTER_BUSINESS_PORT = NO
ROUTER_COVERAGE_COMPOSED_FROM = P8_T
LOWER_SEAM = CONCRETE_HOST_ROUTER_SESSION
```

## 1. Baseline and verdict

- baseline：
  `bc15346042a9000b0fdd9b18bbf0802e63b262c2`
  （tree `2d76f1e13c2b9bca5010fcf6346489f09a845522`）
- branch：
  `codex/p5-f445h-i7-p8-s2-stream-arg`
- integration owner：
  `/root/phase05_integration_steward`

在未修改production的candidate上，新增的独立`kind: test` service通过compiled、linked、admitted
RuntimeAssembly和concrete Host router-session lower seam执行：

```text
RuntimeHost::dispatch_router_binary_frame
  -> overlay entry()
  -> helper/wrap(source())
  -> concrete rawHttp serverStream response sink
```

本fixture没有启动standalone Router进程，也没有监听或访问Router business port。独立Router ordinary
ingress由同一lane已验收的`P5-F445H-I7-P8-T-http-entry-combined-probe-result.md`证明；本任务未修改
T覆盖的Router/Runtime ingress owner，因此S2与T可以组合覆盖入口和下层stream argument，但不能把S2单个
请求表述为经过standalone Router。

normal、producer error和consumer cancel三个case全部GREEN；normal又在撤回trace后连续两次GREEN。合同规定只有
normal连续稳定得到首次`next`的`unknown Stream value`才允许运行dependency-local对照和修改production。
本任务没有得到RED，因此没有移动`source()`、没有公开`wrapLocal`，也没有修改任何Eval owner。

## 2. Fixture shape

保留S1原consumer不变，并在同一fixture tree增加：

- production dependency `argument-provider`，`api.yml`只公开`wrap`；
- 独立`kind: test` service `argument-tests`，以普通public alias `helper`依赖provider；
- `http.yml`只引用本服务`entry.test.skiff`的overlay handler；
- normal源码保持权威任务指定的`source() -> wrap(source()) -> entry`形状；
- producer error在发出`before-error`后抛错；
- consumer cancel在发出`first`后进入长等待，由真实request.cancel中断。

三个case都从linked/admitted rawHttp `serverStream` route构造runtime binary frame，经
`RuntimeHost::dispatch_router_binary_frame`进入Host；测试没有直接调用handler、手工构造Interpreter、
mock response sink或新增测试专用bridge。
测试接收的是Host dispatch实际写入的`RouterWriterMessage`通道，不是network socket。

## 3. 临时trace

临时task-local trace记录了create、每次lookup/next、cancel、finish、request scope和producer executable。
指针在本result中统一去敏为`registry=A`；三个独立case都使用request generation `1`，且各自的三个stream始终
属于同一registry。

三类stream的稳定对应关系为：

| role | stream | owning executable |
| --- | --- | --- |
| HTTP entry producer | `stream-0` | overlay package，`entry.__test.entry` / `producerErrorEntry` / `consumerCancelEntry` |
| dependency `wrap` producer | `stream-2` | provider build `skiff-package-build-v10:sha256:23351201ebc6375eb78d4d68d69ac28248db683e51c90f4de0bb1baa7411c2c7`，package slot 0，module `main`，symbol `main.wrap`，executable 0 |
| overlay-local argument producer | `stream-1` | overlay build `skiff-package-build-v10:sha256:ac4afd7c5e00084fd5f20a93c7e5b55d6267841c60603251ad9cd0eb3ae7d09b`，package slot 1，module `entry.__test`，normal/error/cancel分别为`source`(0)、`producerErrorSource`(2)、`consumerCancelSource`(4) |

normal的关键顺序：

```text
scope-open  A/1 active=0
create      stream-0 A/1 active=1
next        stream-0 A/1
create      stream-1 A/1 active=2
create      stream-2 A/1 active=3
next        stream-2 A/1
next        stream-1 A/1
finish End  stream-1 active=2
finish End  stream-2 active=1
response    start(200), chunk("body"), end；无第二terminal
cancel      stream-0
finish      stream-0 active=0
scope-close A/1 active=0
```

producer error先保留已发chunk，再沿三个producer逐层形成唯一error terminal：

```text
stream-1 finish Error
stream-2 finish Error
stream-0 finish Error
active=0
response start(200), chunk("before-error"), error；无第二terminal
```

consumer cancel在收到`start(200)`和`chunk("first")`后向同一Host dispatch seam发送真实runtime
`request.cancel` binary frame：

```text
stream-0 finish Cancelled
request scope close drains stream-1 / stream-2
active=0
后续source/wrap cancellation cleanup命中已关闭id，不产生晚到response
```

这证明runtime request cancellation，不声称覆盖external socket/client disconnect。

每个case中`stream-1`只创建一次，normal只产生一个`"body"` chunk，证明nested argument producer没有重复消费。
所有临时trace、环境开关和日志协议均已撤回，最终production和fixture不含instrumentation。

## 4. RED/GREEN matrix

| 实验 | 结果 | 后续 |
| --- | --- | --- |
| overlay-local `source()`作为dependency `wrap`参数，normal | Host lower seam GREEN，连续稳定，无`unknown Stream value` | 禁止进入RED-only对照 |
| producer error | GREEN，已发item保留，单error terminal，active归零 | fixture保留 |
| consumer cancel | GREEN，取消传播，active归零，无晚到response | fixture保留 |
| dependency-local `wrapLocal()`对照 | 未运行 | 合同明确禁止 |
| production stream argument实现 | NO-OP | 没有单一偏离symbol |
| S1普通PackageDirect return stream | GREEN | 回归保持 |

## 5. Actual write set

```text
runtime/host/src/host/router_session/tests/runtime_assembly_request.rs
runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs
test-runner/fixtures/package-direct-http-stream-registry/argument-provider/**
test-runner/fixtures/package-direct-http-stream-registry/argument-tests/**
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-S2-stream-producing-argument-transport-result.md
```

没有修改`runtime/eval` production、registry、protocol、schema、compiler、Router、test-runner production、std或
Internals。

## 6. Validation

执行：

```text
cargo test --locked -p skiff-runtime-host \
  package_direct_stream_producer_argument_real_gateway -- --nocapture
```

撤回trace后连续两次均为`1 passed; 0 failed`；单个测试内部顺序执行三个case。
selector中的`real_gateway`是历史名称，只表示concrete Host gateway/session与真实response sink，不表示
standalone Router或business port。

```text
cargo test --locked -p skiff-runtime-host \
  package_direct_http_stream_registry_return_stream_reaches_real_gateway -- --nocapture
```

结果`1 passed; 0 failed`。

```text
cargo test --locked -p skiff-runtime-eval stream_producer_arg -- --nocapture
```

命令成功，但当前selector命中`0`个测试（unit `420 filtered out`，其余integration suites也为`0`命中）；没有把
零命中伪报为行为证据。

其它检查：

| 命令 | 结果 |
| --- | --- |
| `cargo check --locked -p skiff-runtime-eval -p skiff-runtime-host` | PASS |
| `rustfmt --edition 2021 --check <本任务两个Rust写入文件>` | PASS |
| `git diff --check` | PASS |
| `cargo fmt --all -- --check` | baseline-known FAIL，仅命中本任务未修改的`compiler/tests/package_imports.rs`三处A1 RED格式差异 |

没有把`cargo fmt --all`产生的越界机械改动保留在task branch；最终实际写集中的Rust文件均通过聚焦
`rustfmt --check`。

未运行I、完整AIHub、J生态gate、stable/live/network/Mongo/OAuth/browser。

## 7. Handoff

S2证明当前candidate在concrete Host lower seam已能正确运输overlay-local stream-producing argument；
与T组合后覆盖standalone Router ordinary ingress，但没有形成一条穿过Router business port的S2单请求证据。
它没有证明S3的`std.http.emitResponseStream` response-sink传播。顺序状态为：

```text
S2_COMPLETE = YES
S3_UNBLOCKED = YES
I_RESUME_UNBLOCKED = NO
```

只能由S3在自己的独立实验完成后决定是否恢复I。
