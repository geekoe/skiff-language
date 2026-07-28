# P5-F445H-I7-C Compiler ingress consumer

状态：`IMPLEMENTED_PENDING_RESULT`。

## 1. Parent chain and baseline

直接父节点：

- `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md`；
- `P5-F445H-I7-D0-service-scoped-ingress-design-result.md`。

唯一架构事实源为
`doc/architecture/package-service-contract-deployment.md`。D0已经冻结Host不参与Skiff service route；
K已经把canonical `IngressSelector`硬切为`protocol + method + path`，并把
ServiceDeploymentInput/ServiceDeployment/DeploymentArtifact代际切到v5/v4/v4。

| 项 | 值 |
| --- | --- |
| baseline commit | `1a11328a241b5d177eb40885e294fe31d65a7240` |
| baseline tree | `ca1f7c2f040458df4275d00801eb0fc61046a1a8` |
| leaf branch | `codex/p5-f445h-i7-c-compiler-ingress` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-c-compiler-ingress` |
| integration owner | `/root/phase05_integration_steward` |

零worktree只读预检确认：

- authoring DTO的旧HTTP `host` owner位于`artifact-model/src/ecosystem_authoring.rs`；
- `http.yml`/`websocket.yml`严格解析与同service route判重位于
  `compiler/input/src/service_config.rs`；
- deployment ingress projection位于
  `compiler/driver/http_gateway_projection/**`与
  `compiler/driver/websocket_gateway_projection.rs`；
- compiler聚焦测试与fixture可以完整证明本consumer，不需要修改assembly resolver/loader/linker、
  runtime Host/eval、Router TypeScript或K canonical owner。

## 2. Write ownership

本任务严格限于：

```text
artifact-model/src/ecosystem_authoring.rs
compiler/input/src/service_config.rs
compiler/driver/http_gateway_projection/**
compiler/driver/websocket_gateway_projection.rs
compiler/tests/http_gateway_projection.rs
compiler/tests/websocket_ingress.rs
compiler/tests/fixtures/**                 # 仅直接消费http.yml/websocket.yml的fixture
artifact-model/src/**                     # 仅ecosystem authoring直接单测
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-C-compiler-ingress-consumer.md
  P5-F445H-I7-C-compiler-ingress-consumer-result.md
```

禁止修改deployment assembly resolver/loader/linker、runtime Host/eval、Router TypeScript、Internals、
official packages或K已经完成的canonical selector/schema/identity owner。若实现必须越界，立即停止并上报。

## 3. Required implementation

1. `HttpGatewayEntryAuthoring`删除`host`，并继续严格拒绝未知字段；旧`host` authoring必须报错，不能忽略、
   默认或兼容读取。
2. `http.yml` route只由`method + path`组成；method继续规范化，path继续严格校验。
3. 同一service的HTTP重复`method + path`仍失败；不同service的相同route不在authoring/compiler层做全局判重。
4. WebSocket route只由`path`组成，JSON-RPC method仍属于同一connection内部的method声明；projection不再
   构造Host。
5. HTTP/WebSocket projection产出K后的无Host `IngressSelector`，并自然进入当前
   ServiceDeploymentInput v5 / ServiceDeployment v4 / DeploymentArtifact v4链路。
6. 刷新本owner直接消费的测试与fixture；不刷新assembly/runtime/router跨域golden。

## 4. Verification

先锁定旧Host的真实RED，再实现并验证：

```bash
cargo test -p skiff-compiler-input service_config
cargo test -p skiff-compiler --test http_gateway_projection
cargo test -p skiff-compiler --test websocket_ingress
cargo test -p skiff-artifact-model ecosystem_authoring
cargo fmt --all -- --check
git diff --check
```

完成时反向搜索：

- production authoring/projection不再读取、默认或构造route Host；
- 旧`host`字段有明确strict-rejection测试；
- HTTP同service重复`method + path`仍失败；
- 写集未越过owner。

不运行stable/live/network/Mongo/OAuth/browser，不push。
