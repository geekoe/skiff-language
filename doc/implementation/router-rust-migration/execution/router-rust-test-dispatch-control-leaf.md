# Router Rust Migration — Test-Dispatch Control Leaf Task

日期：2026-08-03

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。
节点：test-dispatch control（一次性有界会话）
基线：`feat/router-rust-final-fix@89b0c52f`
Worktree：`/Users/geek/workspace/wt-final-fix`
集成目标：origin/main（`git push origin HEAD:main`，fast-forward）

## 引用

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md` §7 E-http
  （CORS preflight/service-managed/platform error 和 test-dispatch isolation）。
- TS parity corpus：`router/src/router/assemblyControlPlane.ts`（b9714d7f^ 版本，
  即 TS Router 删除前最后状态；本地 main 并行线同文件逐字节一致）。
- E-dispatch gate leaf：`doc/implementation/router-rust-migration/execution/router-rust-e-dispatch-gate-leaf.md`
  （等价 seam = 生产 `DispatcherHttpPort`）。
- W-http leaf：`doc/implementation/router-rust-migration/execution/router-rust-migration-w-http-leaf.md`
  （test-dispatch correlation：`x-skiff-test-case-capability` 等）。

## 零 worktree 只读预检结论（锚定 89b0c52f）

1. 第三环症状确认：runtime/control listener（`router/src/listener.rs`）只路由
   `/runtime` WS、`/__router/health`、`/__skiff/activate-assembly`；其余 HTTP
   路径（包括 `/__skiff/test-dispatch`）回落 200 空 body。`cargo test -p
   skiff-test-runner` 的 `http_entry_test_service` 依赖 test-dispatch。
2. 控制面语义（TS corpus，b9714d7f^ 与本地 main 逐字节一致）：
   `POST /__skiff/test-dispatch`，body 为 runtimeAssembly test dispatch
   （`kind: "test"` + `routing` + `mode` + `httpRequest` + `payloadBase64` +
   `timeoutMs`）；exact fields；canonical Base64；positive safe timeout；
   exact active assembly generation + gateway binding；构建 `request.start`
   帧（`testEffectsEnabled: true` + 新 `testCaseCapability`，无 parent）；
   经 `dispatchAssemblyTestBinary` 返回 runtime 帧，200 `{ok, header,
   payloadBase64}`；解码/绑定失败经 `classifyActivationError` 映射
   `AssemblyActivationRejected`/`AssemblyParticipantsUnavailable`；非 POST →
   405 + `allow: POST`。
3. transport codec：`runtime/transport` 已有完整
   `RuntimeAssemblyRequestStartFrameHeader`/`response.end`/`response.error`
   严格 codec 与 `decode_runtime_assembly_request_start_frame` 语义校验；
   `package_test_dispatch` 帧字段与 corpus 均在（transport 侧无缺口）。
4. runtime capability `package_test_dispatch` 语义：TS Router 删除前最后状态
   （b9714d7f^ `runtime/host/src/host/control_plane.rs`）已为 `false`
   （legacy package-test host seam 移除后）；新的 runtimeAssembly test
   dispatch 路径不检查该 capability（TS `AssemblyRuntimeRegistry.
   pickAssemblyTestDispatchConnection` 只做 exact validation +
   pickHealthyDispatchConnection）。当前 Rust runtime `false` 即 TS parity，
   无需 runtime 改动、无需新增 config 键。
5. W-http correlation 现状：public gateway 已支持
   `x-skiff-test-case-capability`/`x-skiff-test-case-parent-request-id`
   进入 `testEffectsEnabled`/`testCaseCapability`/`testCaseParentRequestId`
   并从 `httpRequest.headers` 剥离（W-http leaf 已完成）；本节点补 control
   listener 端点，public gateway 增加 `/__skiff/test-dispatch` 隔离拒绝。
6. 等价 seam：生产 `HttpDispatchPort`（`supervisor/http.rs`）已有
   `dispatch_unary`，但把 runtime `response.error` 帧与 submit rejection
   混在 `HttpDispatchError` 中；为保持 TS 响应/错误码 parity，本节点在
   `HttpDispatchPort` 增加帧级 `dispatch_test` 方法（生产 adapter 与
   `FakeHttpDispatcher` 同步实现）。
7. 验证中发现并修复的 enabler（activation control lane，最小修复）：
   `ActivationHttpHandler` 等待「首个与 enqueue 时 phase 不同的 terminal」。
   coordinator 在上一事务 commit 后保持 phase=Committed（`tx`/activationId
   保留到下一次 start），因此连续两次 commit 时第二次的 terminal phase 与
   enqueue 时相同，wait 永不满足 → control 请求挂到 HTTP deadline。真实
   isolated 探针证据：第二次 activation 的 runtime 侧 `assembly_committed`
   (generation 2) 已发生，但 `/__skiff/activate-assembly` HTTP 无响应。
   修复：wait predicate 增加 exact activationId 匹配（连续 commit 判别），
   保留 phase-diff 使 pre-`tx` 失败（如 stale expected generation）快速
   收敛；新增 back-to-back commit 回归测试。

## 写入集（全部在本节点授权范围）

- `router/src/test_dispatch/mod.rs`、`router/src/test_dispatch/http.rs`：
  `TestDispatchHttpHandler`（strict decode、exact binding、`request.start`
  构建 + transport 语义校验、`dispatch_test` 路由、TS parity 响应/错误码）。
- `router/src/http/dispatch.rs`：`TestDispatchOutcome` + `HttpDispatchPort::dispatch_test`。
- `router/src/supervisor/http.rs`：`DispatcherHttpPort::dispatch_test`（生产
  frame 级实现，re-emit `response.end`/`response.error`）。
- `router/src/http/fake.rs`：`FakeHttpDispatcher::dispatch_test`。
- `router/src/http/ingress.rs`：`http_surface_view_from_epoch` 改 pub(crate)
  （test-dispatch live surface 复用）。
- `router/src/http/frame.rs`：trace/span/deadline 生成器 pub(crate) +
  `new_test_case_capability`。
- `router/src/http/server.rs`：public gateway `path_is_control` 增加
  `/__skiff/test-dispatch`（404 `ControlEndpointNotFound` 隔离）。
- `router/src/listener.rs`：runtime/control listener 路由 test-dispatch +
  6-arg additive 启动 seam。
- `router/src/supervisor/mod.rs`：production 装配 `TestDispatchHttpHandler`。
- `router/src/lib.rs`：`pub mod test_dispatch` + re-exports。
- `router/src/activation/http.rs`：back-to-back commit enabler 修复 +
  回归测试（见预检 7；仅 wait predicate 与测试 fixture）。
- 测试：`router/src/test_dispatch/http.rs` 单元测试（10 个）、
  `router/tests/test_dispatch_control.rs` 真实 socket 端点探针（2 个）、
  `router/src/activation/http.rs` back-to-back commit 回归测试（1 个）。
- 本叶子文档。

## 禁止写 / 非目标

- 不改 runtime crate（capability 维持 TS parity 的 `false`，见预检 4）。
- 不改 deployment、AGENTS.md、scripts README、verify 文件、skiff-instance.mjs。
- 不操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量 verify。

## 完成标准与验证

1. `cargo test -p skiff-router` 全绿（含新单元测试 + 端点探针，无回归）。
2. `cargo test -p skiff-test-runner` 全绿；`http_entry_test_service` 必须通过
   （EXPECTED_CONCURRENCY_REJECTION / HAPPY_HTTP_ENTRY_PASS /
   ASSEMBLY_READY / ISOLATED_CLEANUP_PASS）。
3. 负例：`rg` 确认 public gateway 对 `/__skiff/test-dispatch` 返回
   `ControlEndpointNotFound`；control listener 非 POST 405。
