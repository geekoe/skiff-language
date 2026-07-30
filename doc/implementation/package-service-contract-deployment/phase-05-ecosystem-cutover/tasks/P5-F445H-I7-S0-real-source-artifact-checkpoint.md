# P5-F445H-I7-S0 real-source / artifact checkpoint

状态：`READY`。本节点严格执行 I7R 冻结的 S0 Skiff source/artifact 检查点；不重写
package/service、current execution scope、timeout、HTTP、WebSocket、Actor、service call 或
artifact identity 语义。

## 1. 直接父节点与权威链

直接执行父节点：

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`
- `P5-F445H-I6K-R4-independent-current-scope-reacceptance-result.md`
- `P5-F442C-cross-system-corpus-verifier-closeout-result.md`
- `P5-F443B-cheap-combined-executable-resume-result.md`
- `P5-F444C-agine-service-terminal-connect-only-cutover-result.md`
- `P5-F445C-package-interface-identity-normalization-result.md`
- `P5-F445H-I6S-service-timeout-scope-reduction-result.md`

这些父节点继续经 Phase 05 DAG 追溯到唯一权威设计：

```text
doc/architecture/package-service-contract-deployment.md
```

I6K-R4 已给出 `I6_ACCEPTED = YES`、`I7_UNBLOCKED = YES`。I7R 冻结的 S0 是
compiler/test-runner fixture owner；S1、Internals Agine（A）与 Codex Relay/AIHub（C）等待本
节点的 current source/artifact receipt。

## 2. 精确输入与执行身份

| 项 | 值 |
| --- | --- |
| baseline commit | `54fb087f122c53aed5c017260c7bca43e2b54404` |
| baseline tree | `008d3a05927cdf845004db980d1b46de263612be` |
| branch | `codex/p5-f445h-i7-s0-source-artifact` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-s0-source-artifact` |
| integration owner | `/root/phase05_integration_steward` |
| official-package provenance input | `19cfab5dfc827450d37e1a103d21f31f8effa4f0` / tree `44081bd0498919086c13adea97c07722cb768352` |

开始前的 baseline 反向搜索确认 tracked `.skiff` source 中 `timeout(` 为零；现有 timeout
compiler test只使用临时 source，Host/Router tests则使用 Rust-generated source或 synthetic
artifact。故当前没有 checked-in source 能建立 S0 纵向 receipt。

## 3. Owner、真实入口与写入范围

Production owner不变；本节点只拥有 checked-in source fixture 与聚焦 test harness。

允许 tracked 写入：

```text
test-runner/fixtures/package-service-current-scope/**
test-runner/tests/package_service_contract_deployment.rs
test-runner/src/canonical_package/tests/combined.rs
P5-F445H-I7-S0-real-source-artifact-checkpoint.md
P5-F445H-I7-S0-real-source-artifact-checkpoint-result.md
```

`combined.rs`只允许机械刷新 F76 official-package test root provenance，从 package root改读
current `tests/<package>` test-service roots；不得改变 compiler/package semantics。

禁止写入：

- compiler、runtime、Host、Router、artifact model/identity、protocol/schema production；
- 其它 Skiff test fixture owner；
- Internals或`skiff-packages`；
- stable/live脚本、实例配置、MongoDB、network/OAuth/browser状态；
- 权威设计与runtime reference。

机械 caller、fixture和 test selector若因果上直接相关可自主闭合并记录；若需要共享 production
owner、公共契约、DAG或identity语义变化，立即停止并上报。

## 4. 实现与可观察完成标准

新增一个真实 multi-root fixture：

```text
package-service-current-scope/
  helper/
  provider/
  consumer/
```

consumer必须使用 current `package.yml`、`api.yml`、`service.yml`、`http.yml`、
`websocket.yml`；真实 `.skiff` source在嵌套 `timeout(...)` 中覆盖：

- outbound HTTP unary与stream；
- outbound WebSocket `requestJsonToConnection`，严格三个业务参数；
- file operation；
- Actor operation；
- first-version canonical service call，只继承 caller current scope，不读取 deployment timeout。

复用现有 canonical authoring producer发布 provider、consumer、deployment与assembly。聚焦 receipt
必须从同一 checked-in source读取并证明：

1. current File IR schema/format/opcode identity；
2. exact PackageArtifact/build/local ABI；
3. exact ServiceContract/ServiceProtocol；
4. exact ServiceDeployment/DeploymentArtifact；
5. HTTP unary、HTTP server-stream与WebSocket GatewayEntry；
6. exact RuntimeAssembly；
7. artifact store round-trip后仍为同一 typed records；
8. inline ingress、old call spelling与 timeout fact mutation fail closed或改变正确 identity，而
   public ABI保持不变。

F445C 已拥有 package/interface owner positive/negative矩阵；S0不得复制或放宽它。S0完整 compiler
selector会保留该父证据，新增 source mutation只验证本fixture的service-call spelling和artifact
identity边界。

## 5. 证据 owner与命令

开发 owner只运行以下快速/聚焦证据：

```bash
CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo test -p skiff-compiler --test timeout_artifact_lowering --locked

CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment \
  canonical_live_source_roots_compile_to_current_receipts --locked

P5_F76_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
  CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  cargo test -p skiff-test-runner p5_f76_contextual_callable_provenance_combined \
  --locked -- --ignored

CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=build/cargo-target \
  node scripts/verify.mjs --only compiler,test-runner

cargo fmt --check
git diff --check
```

S0 owner不运行全 Skiff gate、stable/live/network/MongoDB。`node scripts/verify.mjs --only
compiler,test-runner`是本节点唯一完整组件 owner；最终全仓 gate属于 J。

证据只对本 branch最终 implementation commit/tree、official-package prepared input与
worktree-local Cargo target有效。I6 production、compiler lowering/schema/identity、fixture
source、official test-root layout或验证工具变化都会使相关 receipt失效。

## 6. 风险、成熟度与停止条件

风险：中高。修改仅是fixture/tests，但receipt跨 source/compiler/artifact/assembly identity边界。
当前候选是 I7 implementation checkpoint；完成后建立 `S0_COMPLETE` 并解除 S1、A、C，不升级为
稳定候选或阶段完成。

必须停止并报告 `TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE`：

- checked-in source需要新增语法、native/public API、schema、identity或compatibility path；
- current canonical authoring无法在既有production上形成单一 exact receipt；
- 需要修改runtime/Host/Router production才能让S0通过；
- 与运行中兄弟节点发生共享 fixture/test owner冲突；
- package/interface owner、service timeout或公开 cancellation语义需要重开；
- 聚焦失败证明存在独立production owner，而非本fixture/test harness机械缺口。

Implementation（task/fixture/tests）与 result分别提交；不得merge、rebase、push或清理一级
worktree/branch。完成后向 integration owner直接交付 branch/worktree、commit/tree、实际写集、
证据与解除节点。
