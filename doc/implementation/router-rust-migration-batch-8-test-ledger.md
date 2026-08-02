# Router Rust Migration Batch 8 — W-differential Test Ledger

日期：2026-08-02
归属：W-differential（baseline `origin/main@d228b613`）

按权威设计 `doc/implementation/router-rust-migration-plan.md` §9：每删除
一个 TS test，ledger 必须标记为以下四类之一；不能以类型系统代替
observable test。

## 处置分类

| 分类 | 含义 |
| --- | --- |
| `retired` | 行为/端点/契约被删除，无对应 observable test 需求；删除行为本身即替代 |
| `shared owner` | 测试保留且 re-owned 到新的 canonical owner（文件/用例移动或改造） |
| `Rust replacement` | 行为由 Rust 实现/测试替代，TS 侧删除 |
| `black-box replacement` | 保留 black-box 级可观察替代（外部 HTTP/WS/Mongo/进程边界），实现内部不可见 |

## 本节点删除记录

本节点（W-differential）**未删除任何 router TS test**。d228b613 之前各批次
的删除已由 `router-rust-migration-batch-1-test-ledger.md`（C0-control）等
批次文档覆盖；本 ledger 自 batch 8 起成为 Router 迁移 TS test 处置的
canonical 登记处。

## Baseline 审计（d228b613，全部 retained）

以下 66 个 router TS test 文件在 d228b613 均存在且未被本节点改动；任何
后续删除（A2 等）必须把对应行从 retained 改为四类处置之一并填理由/替代
owner。`loop-risk-health.test.ts` 在 batch-1 已按 shared owner 改造并保留，
仍计入本表。

| 测试文件（d228b613 baseline） | 处置 |
| --- | --- |
| `router/tests/active-assembly-reload.test.ts` | retained |
| `router/tests/actor-get-create-activation.test.ts` | retained |
| `router/tests/actor-manager.test.ts` | retained |
| `router/tests/actor-owner-lease-idle-ttl.test.ts` | retained |
| `router/tests/actor-production-routing.test.ts` | retained |
| `router/tests/actor-router-admission.test.ts` | retained |
| `router/tests/actor-runtime-disconnect.test.ts` | retained |
| `router/tests/actor-spawn-correlation-lifecycle.test.ts` | retained |
| `router/tests/actor-spawn-submit.test.ts` | retained |
| `router/tests/actor-test-capability-authority.test.ts` | retained |
| `router/tests/actor-test-capability-session-race.test.ts` | retained |
| `router/tests/actor-test-capability-terminal-lifecycle.test.ts` | retained |
| `router/tests/actorMethodProtocol.test.ts` | retained |
| `router/tests/assembly-activation-service-db-wire.test.ts` | retained |
| `router/tests/assembly-http-gateway-cors.test.ts` | retained |
| `router/tests/assembly-http-gateway-stream.test.ts` | retained |
| `router/tests/assembly-replica-dispatch.test.ts` | retained |
| `router/tests/assembly-runtime-endpoint.test.ts` | retained |
| `router/tests/compilerGeneratedManifestCompatibility.test.ts` | retained |
| `router/tests/config-corpus.test.ts` | retained |
| `router/tests/config-view.test.ts` | retained |
| `router/tests/config.test.ts` | retained |
| `router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts` | retained |
| `router/tests/h_registration_cut_handshake.test.ts` | retained |
| `router/tests/h_spawn_parent_cut_parent_kind.test.ts` | retained |
| `router/tests/h_spawn_parent_cut_spawn_wire.test.ts` | retained |
| `router/tests/host-ingress.test.ts` | retained |
| `router/tests/http-telemetry.test.ts` | retained |
| `router/tests/json-rpc-20-text-profile.test.ts` | retained |
| `router/tests/loop-risk-health.test.ts` | retained |
| `router/tests/manifest-validation.test.ts` | retained |
| `router/tests/mongo-assembly-activation-state-store.test.ts` | retained |
| `router/tests/pathPattern.test.ts` | retained |
| `router/tests/protocol.test.ts` | retained |
| `router/tests/publicationId.test.ts` | retained |
| `router/tests/raw-http.test.ts` | retained |
| `router/tests/release-routing.test.ts` | retained |
| `router/tests/router-bootstrap-session.test.ts` | retained |
| `router/tests/router-control-plane.test.ts` | retained |
| `router/tests/router-websocket-trust-dispatch.test.ts` | retained |
| `router/tests/runtime-assembly-actor-catalog.test.ts` | retained |
| `router/tests/runtime-assembly-request-wire.test.ts` | retained |
| `router/tests/runtime-assembly-unary-dispatch.test.ts` | retained |
| `router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts` | retained |
| `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts` | retained |
| `router/tests/runtime-assembly-websocket-rpc-snapshot.test.ts` | retained |
| `router/tests/runtime-bootstrap-protocol.test.ts` | retained |
| `router/tests/runtime-capability-session-fence.test.ts` | retained |
| `router/tests/runtime-dispatch-deadline.test.ts` | retained |
| `router/tests/runtime-dispatcher-self-ingress-actor-parent.test.ts` | retained |
| `router/tests/runtime-endpoint-actor-message-fifo.test.ts` | retained |
| `router/tests/runtime-endpoint-connection-send-trust.test.ts` | retained |
| `router/tests/runtime-endpoint-source-lifecycle.test.ts` | retained |
| `router/tests/runtime-errors.test.ts` | retained |
| `router/tests/runtime-protocol-websocket-response.test.ts` | retained |
| `router/tests/runtime-registry-dispatch.test.ts` | retained |
| `router/tests/service-deployment-selection.test.ts` | retained |
| `router/tests/service-error-cross-layer-convergence.test.ts` | retained |
| `router/tests/strict-json.test.ts` | retained |
| `router/tests/test-dispatch-lazy.test.ts` | retained |
| `router/tests/test-dispatch.test.ts` | retained |
| `router/tests/websocket-connection-lifecycle.test.ts` | retained |
| `router/tests/websocket-gateway.test.ts` | retained |
| `router/tests/websocket-generation-lifecycle-router.test.ts` | retained |
| `router/tests/websocket-generation-lifecycle-wire.test.ts` | retained |
| `router/tests/websocket-jsonrpc-gateway.test.ts` | retained |
| `router/tests/websocket-request-broker.test.ts` | retained |
| `router/tests/websocket-rpc-bridge.test.ts` | retained |

## 删除登记协议

删除任何 router TS test 的 commit 必须同时：

1. 在上表中把对应文件行改为 `retired` / `shared owner` / `Rust
   replacement` / `black-box replacement` 之一；
2. 填写"理由"与"替代 / 新 owner"列（指向 Rust test、保留文件或删除语义）；
3. 若属于 black-box replacement，确认对应 differential scenario
   （`scenario-inventory.json`）已声明该可观察面并真实跑通；
4. 不得仅以"类型系统/编译通过"代替 observable test 记录。
