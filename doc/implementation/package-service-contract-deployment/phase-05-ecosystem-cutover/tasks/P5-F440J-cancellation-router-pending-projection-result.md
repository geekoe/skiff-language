# P5-F440J Cancellation Router pending / projection follower result

状态：`COMPLETED`。未触发 `TASK_SCOPE_EXPANDED`。

Router 已从 ordinary runtime/service error 公开面删除 cancellation：

- fixed-service `platformError` 不再接受历史 cancellation builtin identity；
- control `response.error` 将同一历史 code 作为 internal-cancellation reserved code 硬拒绝；
- `RuntimeResponseError` 不再把该 code 投影为 HTTP 499；
- 没有新增 replacement code、alias、fallback 或兼容路径。

`request.cancel`、bounded reason、pending owner、TimeoutError、ProviderUnavailable 及 client-closed
telemetry 499 均保留。实现没有修改 Rust、runtime/model、compiler、artifact、scripts、fixture 或
service root。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 精确 integration 输入 | `5e26079dbdc851e3528d3ad8dbf809cf9b7fd29c` | `23cba855509ad4509215458125d97fadcf93d686` |
| task worktree 起点 | `10e0a16bec077c68a5832ac025505a08fd68d31c` | `bfcf6d0ecf6bccb99453c71d9d00cd9c6472cfd8` |
| implementation | `72c1294207d3fdf763459b3c8759dbe325309690` | `a3e111184cee3e5751540a81993f7a53dcf5f740` |

task 起点相对精确 integration 输入只新增 F440J 任务文档。implementation 精确修改：

- `router/src/protocol/runtimeProtocol.ts`
- `router/src/router/errors.ts`
- `router/tests/protocol.test.ts`
- `router/tests/runtime-assembly-unary-dispatch.test.ts`

除此之外只新增本文 result。

## 2. 实现结果

### 2.1 Ordinary error hard cut

- `PLATFORM_SERVICE_ERROR_IDENTITIES` 删除 cancellation member，因此 fixed-service
  `PlatformError` payload 在 `decodeServiceErrorEnvelope` 阶段 fail closed。
- control `response.error` 仍允许现有 infrastructure error code，但新增一条精确负向 tombstone：
  historical cancellation code 是 internal-cancellation reserved code，不能进入
  `ValidatedResponseErrorFrame`。
- tombstone 从两个语义片段构造，production 不重新注册或写出历史 public spelling，同时仍能在边界
  拒绝旧 runtime payload。
- `runtimeErrorStatus` 删除 cancellation 到 499 的分支。直接构造该非法 ordinary payload 只能落入
  generic 500；真实 endpoint 会更早关闭违规 runtime，并让仍活 caller 得到
  `ProviderUnavailableError`。
- `TimeoutError -> 504` 与 `std.service.ProviderUnavailableError -> 503` 保持不变。

### 2.2 Control plane 与 pending owner

没有新建第二个 pending 状态机。现有 `RuntimeDispatcher.finishPending` 继续是唯一 terminal owner：

- timeout、caller abort、client disconnect、backpressure、protocol/callback error 与 Router shutdown
  先 detach pending，再最多发送一次适当 `request.cancel`；
- runtime-originated `request.cancel` 与 runtime disconnect 不回送 cancel；
- duplicate cancel、late `response.end`、late `response.error` 和 disconnect 都因 pending 已删除而无效；
- fixed/control error channel继续互斥。

新增真实 RuntimeEndpoint + RuntimeDispatcher + HTTP gateway 探针直接冻结这些事实，没有只测试孤立 helper。

## 3. 测试先行

### 3.1 Selector listing

使用 integration worktree 已存在的 Router `node_modules` 临时只读链接；未安装依赖。先执行：

```bash
pnpm --dir router exec vitest list \
  tests/protocol.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts
```

最终列出 70 个非零 selector：protocol 50 个、RuntimeAssembly unary 20 个。临时链接在验证后已删除。

### 3.2 Red evidence

production 修改前执行：

```bash
pnpm --dir router exec vitest run \
  tests/protocol.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  -t 'rejects legacy cancellation|does not project the rejected legacy cancellation code'
```

结果：2 个测试失败、67 个 skipped；共 3 条直接失败断言：

1. fixed-service cancellation platform identity 被接受；
2. control cancellation error code 被接受；
3. `RuntimeResponseError` 把该 code 投影为 499。

失败精确命中待删除公开面，不是零测试、环境错误或无关失败。

### 3.3 Green matrix

| 命令 / selector | 结果 |
| --- | --- |
| 两文件 cancellation boundary selector | 3 passed、66 skipped |
| unary timeout / mutual exclusion / abort / disconnect / race / late terminal selectors | 7 passed、12 skipped |
| unary Router shutdown selector | 1 passed、19 skipped |
| 五个要求的 focused 文件一次执行 | 5 files、147 passed、0 failed |
| `pnpm --dir router type-check` | PASS |
| `git diff --check` | PASS |

五文件聚焦命令为：

```bash
pnpm --dir router exec vitest run \
  tests/protocol.test.ts \
  tests/runtime-registry-dispatch.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/raw-http.test.ts \
  tests/assembly-http-gateway-stream.test.ts
```

任务给出的 `pnpm --dir router test -- tests/protocol.test.ts` 形式被项目脚本解释为全 Router suite，
意外展开为 50 files / 647 tests，结果全部通过。没有把这次意外展开当作最终昂贵 gate，也没有继续运行
完整 verify、Rust、live、instance 或 stable；最终代码状态由上面的 direct Vitest 五文件 147 条与
type-check 重新验证。

## 4. Terminal、Timeout 与 ProviderUnavailable 证据

| 场景 | 真实结果 | exactly-once / late 证据 |
| --- | --- | --- |
| deployment/platform timeout 三种组合 | HTTP 504 + `TimeoutError`；reason `timeout` | 每个 request 一个 cancel、一次 `finishPending`；late success ignored |
| caller abort | reason `caller_cancel` | duplicate abort仍一个 cancel、一次 finish；late success ignored |
| HTTP client disconnect | 不写 ordinary HTTP response；reason `client_disconnect` | duplicate destroy仍一个 cancel、一次 finish；late success ignored |
| runtime-originated cancel | caller得到 `ProviderUnavailableError`，不生成/转发 ordinary `response.error` | duplicate cancel、late success、late error、随后 disconnect合计一次 finish；Router回送 cancel为零 |
| provider/runtime disconnect | 仍活 caller得到 503 `std.service.ProviderUnavailableError` | 一次 finish；没有误分类成 user cancellation |
| Router shutdown | reason `router_shutdown`，caller得到 `ProviderUnavailableError` | duplicate close仍一个 cancel、一次 finish；late success ignored |
| unary protocol failure | existing `protocol_error` cancel保持 | 一次 terminal，late terminal ignored |
| stream client disconnect / backpressure | existing `client_disconnect` / `backpressure` 保持 | focused raw/assembly stream tests继续证明一个 terminal cancel |
| fixed/control error | 两 channel互斥 | existing real-dispatch test继续通过 |

## 5. Reverse search

```bash
rg -n 'CancelError' router/src router/tests
```

- `router/src/**`：`ZERO_MATCHES`。
- `router/tests/**`：5 行，全部位于三个命名清楚的 negative rejection / no-499 tests；没有 positive
  payload、fixture 或兼容断言。

```bash
rg -n 'request\.cancel|finishPending|499' router/src router/tests
```

分类：

- `request.cancel` production 命中只属于 typed control frame type、双向 protocol validator、
  RuntimeEndpoint control dispatch、RuntimeDispatcher cancel sender/receiver；
- `finishPending` production 命中只属于 canonical pending terminal owner及其各终态入口；test 命中是
  exactly-once spy；
- `router/src/router/httpGateway.ts` 的 499 只在 HTTP connection 已关闭、没有写出普通响应时作为
  telemetry observation；
- `router/src/gateway/webSocketGateway.ts` 的 `4999` 是 WebSocket close-code 上界，与 HTTP error
  projection 无关；
- unary test 标题中的 499 是明确的 negative projection assertion。

## 6. Scope 与交付状态

- 未修改唯一写集 `router/**` 和本文 result 之外的文件。
- 未修改 Rust/runtime/model/compiler/artifact，未发现需要扩张 wire schema owner 的 blocker。
- 未 merge、rebase、push、安装依赖、运行 live/instance/stable 或注册 watch。
- implementation 与 result 分开提交。
