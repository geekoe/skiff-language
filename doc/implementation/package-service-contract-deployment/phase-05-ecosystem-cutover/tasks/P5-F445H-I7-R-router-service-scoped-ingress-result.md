# P5-F445H-I7-R Router service-scoped ingress consumer result

状态：

```text
PASS
R_ROUTER_COMPLETE = YES
CONSUMER_JOIN_READY = YES
DECISION_REQUIRED = NO
ROUTER_BLOCKING_ISSUES = 0
EXTERNAL_VALIDATION_BLOCKERS = 1
```

Router已经按service/version先选择精确deployment，再在deployment内按HTTP method/path或
WebSocket path/method选择入口。HTTP Host只保留为请求metadata，不再参与路由；Router产生的HTTP、
WebSocket connect与WebSocket JSON-RPC runtime frame均使用frame v2并携带完整
`ServiceDeploymentRef`。

## 1. Parent and exact identities

| 项 | 值 |
| --- | --- |
| direct task | `P5-F445H-I7-R-router-service-scoped-ingress.md` |
| direct parent | `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md` |
| authority parent | `P5-F445H-I7-D0-service-scoped-ingress-design-result.md` |
| original baseline commit/tree | `1a11328a241b5d177eb40885e294fe31d65a7240` / `ca1f7c2f040458df4275d00801eb0fc61046a1a8` |
| wire checkpoint commit/tree | `084f41a0c0f5fd816815042a5aedba8d0e8a1b61` / `f1ec585d0ab18570dfa5ffaf1f2573b7ed3564b6` |
| combined validation baseline commit/tree | `b9aaed250d23f522165136a4cfa35b127d0c8826` / `758fc89311b2f7bbfb8f5d9115eb9aa99d78652d` |
| Router implementation commits | `a555cbb7373be27031d73a323f7e4f60c9524591`、`a2207af2baeb5e839116d5a31c318a79ba3d3d00` |
| implementation tree | `beefac0d85b0d147d196f827290cdd91411e9208` |
| branch | `codex/p5-f445h-i7-r-router-ingress` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-r-router-ingress` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree在Git handoff中报告；result不能自引用自己的commit identity。

实现提交已重放到同时包含Wire、Assembly与Compiler consumer的combined baseline，未覆盖并行owner的
改动。

## 2. Implementation

### Exact deployment selection

- 新增严格读取`x-skiff-service`与`x-skiff-version`的公共选择器；缺失、重复、逗号歧义、空白或
  非canonical输入返回400；
- `RuntimeAssemblyIngressIndex`先以`serviceId + contractVersion`选择唯一deployment，再使用
  service-local selector；
- 不同service可共享同一个`GET /v1/models`；同service重复selector和同一service/version出现多个
  deployment revision均fail closed；
- RuntimeAssembly reader同时校验所有resolved deployment coordinate唯一，不允许没有入口的第二个
  revision绕过约束。

### HTTP and WebSocket

- HTTP请求Host仍用于构造、校验origin-form URL与透传请求metadata，但不参与deployment或handler选择；
- HTTP、WebSocket upgrade和WebSocket JSON-RPC三条producer都把精确deployment写入routing；
- WebSocket连接捕获精确deployment与assembly generation，后续method frame不能被另一个deployment
  替换；
- 缺失、非法或未知service/version在WebSocket admission前失败，不发生runtime dispatch或generation
  pin。

### Generation hard cut

- Router只读取RuntimeAssembly v3、ServiceDeployment v4与DeploymentArtifact v4；
- Router runtime producer和共享WebSocket lifecycle fixture使用`skiff-runtime-frame-v2`；
- DeploymentArtifact v4 identity排序不再读取已经删除的selector Host；
- 旧ServiceDeployment v3、旧frame、旧Host selector继续严格拒绝，没有dual-read或fallback。

## 3. RED and GREEN evidence

### Real RED

Wire checkpoint之后、Router producer修改之前，TypeScript type-check直接暴露了旧Router仍读取
`selector.host`且runtime routing缺少deployment的断链。实现过程中首轮全量还分别暴露：

- ServiceDeployment identity排序仍强制读取Host；
- Router reader仍停留在ServiceDeployment v3；
- WebSocket lifecycle共享fixture仍为frame v1 / assembly v2；
- HTTP与WebSocket测试请求没有trusted service/version header。

这些失败均在production或直接fixture owner中修正后转绿。

### Final verification

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| Router type check | `pnpm --dir router type-check` | PASS |
| Router non-blocked full set | `vitest run`排除3个依赖旧test-runner的suite | PASS，806/806 |
| Compiler generated → Router join | `compilerGeneratedManifestCompatibility`首个真实生成/join case | PASS，1/1 |
| Router snapshot/WebSocket/protocol聚焦 | 直接相关suite | PASS |
| Rust transport wire | `cargo test --locked --manifest-path runtime/transport/Cargo.toml runtime_assembly_request` | PASS，19/19 |
| formatting | `cargo fmt --all -- --check` | PASS |
| whitespace | `git diff --check` | PASS |
| stale Router production search | `selector.host`、`ingress.host`、`canonicalIngressHost`、frame v1、assembly v2、deployment v3 | PASS，0 |

WebSocket selector负例覆盖缺失service、缺失version、歧义service、未知service与未知version；service
选择器单测另外验证非canonical version。

## 4. External validation blocker

combined baseline中的test-runner consumer尚未完成Host移除，以下位置仍不能通过Rust编译：

```text
test-runner/src/ecosystem_smoke_fixture.rs:84
test-runner/src/package_test_assembly.rs:255
test-runner/src/runtime_execution.rs:233
```

因此依赖`skiff-package-service-smoke-fixture`启动的
`assembly-http-gateway-stream.test.ts`、`runtime-assembly-unary-dispatch.test.ts`以及
`compilerGeneratedManifestCompatibility.test.ts`第二个case无法执行。错误均为
`IngressSelector`已经没有`host`字段，并非Router失败；其中不依赖test-runner的compiler真实生成/join
case已经通过。

test-runner consumer合入后，integration owner应在combined tree补跑完整
`pnpm --dir router test`，不需要Router侧设计决策。

## 5. Actual write set

实现修改限于：

- Router TypeScript runtime protocol、HTTP/WebSocket gateway、active assembly snapshot/loader/registry；
- Router直接测试与共享runtime request/lifecycle fixture；
- Rust/TypeScript runtime request wire schema与对应golden；
- 本task/result文档。

没有修改compiler authoring/projection、deployment resolver/linker、test-runner、Rust Host
activation、Internals或official packages；没有运行stable/live/network/Mongo/OAuth/browser，也没有
push。

## 6. Handoff

R可以交由`/root/phase05_integration_steward`合入当前consumer candidate。合入test-runner consumer后只需
补跑Router完整suite并记录最终计数。

```text
R_ROUTER_COMPLETE = YES
CONSUMER_JOIN_READY = YES
DECISION_REQUIRED = NO
ROUTER_BLOCKING_ISSUES = 0
EXTERNAL_VALIDATION_BLOCKERS = 1
```
