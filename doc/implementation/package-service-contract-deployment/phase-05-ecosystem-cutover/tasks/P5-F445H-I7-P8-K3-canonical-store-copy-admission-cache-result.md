# P5-F445H I7 P8 K3 canonical store copy admission cache result

状态：

```text
PASS
TASK_SCOPE_EXPANDED = YES
READY_FOR_INTEGRATION = YES
```

## Baseline and commits

- Skiff baseline：
  `90de5a2048f85c308068ec82e15b41966e4ea773`
- baseline tree：
  `fb2f928b602e49e05047333d766397e7e8f95d0d`
- canonical store copy admission：
  `a1358a52d274638973e54ea9091a319f951d9084`
- fixture assemble/publish admission reuse：
  `6c2f538e80d30e362832175516eafb35b668cb0c`
- implementation tree：
  `fd39ca71a164d517959eee1bc9b26436624be496`

范围从最初的canonical store package copy扩展到了同一根因的fixture assemble与test-owned
package publish。扩展发生前已通过真实Agine采样确认：只修copy仍会在case 1前由compiler、
deployment、assembly和test-owned publication重复执行相同package/File IR identity投影。

## Implementation

### Canonical store copy

- 新增字段私有、不可Serde、不可外部构造的`ValidatedPackageCopyRecords`。
- 首次admission完整验证package、schema index/type records、File IR和static resource，并保存：
  - canonical source root；
  - exact `PackageArtifactRef`；
  - record kind与canonical path；
  - exact record bytes和length。
- cache key为`(source root, PackageArtifactRef)`；不同root或ref不能复用。
- cache hit重新从source安全读取每条record并比较length与全部exact bytes；没有仅按identity
  跳过，也不重复对大型File IR做canonical projection或SHA-256。
- target继续走immutable exact-byte write；已有不同内容会fail closed。
- package、schema、File IR或resource在首次admission后被篡改，下一次命中都会失败。

### Fixture assemble

- 新增进程内opaque `ValidatedPackageArtifact`，持有完整验证后的不可变typed artifact、
  exact ref和canonical content snapshot。
- 原有compiler/deployment/assembly公共入口仍保留完整验证行为；只新增显式接收opaque
  admission的内部复用入口。
- 每次复用都先要求raw `PackageArtifact`与token中的完整typed内容严格相等。
- test-service fixture在进入case循环前只对implementation、dependency与base package
  closure各做一次完整package identity admission。
- HTTP/WebSocket gateway、generated deployment、deployment projection、package bindings和
  runtime assembly candidate index消费同一组admission，不再在每个case重建identity。
- 一条专门回归确认不同raw artifact不能借用已验证token。

### Fixture publish

- 一个`CanonicalPublishSession`覆盖全部case。
- external package的package/schema/File IR/resource都通过同一个source-root-bound exact-byte
  token复制。
- test-owned `PublishedPackageArtifact`首次仍走原完整publication；重复发布要求：
  - target root与exact ref相同；
  - 完整`PublishedPackageArtifact`结构相等；
  - target canonical store admission继续与exact bytes一致。
- 仅用于区分owned/external的集合采用declared ref；它不能形成验证旁路，因为owned package
  随后必经上述首次完整publication或已验证重复路径。

## Evidence

```text
cargo fmt --all
PASS

git diff --check
PASS

cargo check --locked \
  -p skiff-artifact-identity \
  -p skiff-deployment \
  -p skiff-compiler \
  -p skiff-test-runner --tests
PASS

cargo test --locked -p skiff-artifact-identity
PASS: 101 passed, 0 failed

cargo test --locked -p skiff-deployment
PASS: 65 passed, 0 failed

cargo test --locked -p skiff-compiler
PASS: all unit, binary, integration and doc tests

cargo test --locked -p skiff-test-runner --lib
PASS: 53 passed, 0 failed, 2 ignored

cargo test --locked -p skiff-test-runner --test test_service_flow
PASS: 17 passed, 0 failed
```

多case回归同时比较单case与三case fixture：

- 完整package identity admission次数相同；
- admission次数等于unique package closure大小；
- 三个case仍生成相互独立的deployment与assembly；
- publish/read round trip保持通过。

storage回归覆盖：

- 同root、同ref只产生一次完整admission；
- 不同identity或不同source root分别admit；
- package、schema、File IR、resource source tamper全部fail closed；
- target exact-byte冲突fail closed；
- invalid首次admission不进入cache。

## Real Agine proof

使用：

- Internals：
  `/Users/geek/workspace/internals-terminal-verifier-probe`
  at `935b1e3208654a9bbfc4c34b71bbbea88c46c61f`
- official packages：
  `/Users/geek/workspace/skiff-packages-phase-05-integration`
  at `730b26a1221e15e97f66729baa6031ef69346633`
- 本结果的Skiff implementation tree。

top-level `runCanonicalFixtureWorkflow`真实生成同一个隔离的ecosystem store，发布7个package、
Relay与AIHub两个dependency service，并生成一个base assembly。随后直接运行Agine test service；
activation指向故意不存在的`127.0.0.1:9`，因此只有在fixture编译、全部case组装和发布完成后才
会得到预期连接失败。

```text
canonical dependency workflow:       58,329 ms
runner -> first case activation:      85,334 ms
total:                               143,663 ms
maximum resident set size:     5,804,965,888 bytes
reachedFirstCaseActivation:             true
```

原始问题为单核运行约12分钟仍未进入case 1。最终probe已进入首case activation，且没有启动
stable instance、MongoDB、runtime/router、OAuth、browser或外部网络。

5.8 GB峰值内存是exact-byte admission保存大型File IR snapshot的明确成本；它不阻塞本节点
正确性与墙钟时间验收，但建议后续单独设计低内存的不可伪造snapshot表示，不能在本节点中退回
到全局identity集合、仅identity跳过或不校验source exact content。

## Known environment-only gate

完整`skiff-test-runner`命令中的
`explicit_test_http_entries_cross_the_real_isolated_router`依赖worktree内`node_modules/tsx`；
当前linked worktree没有该依赖。此前尝试在启动fixture时得到`tsx: command not found`。
本任务未安装依赖，也未用stable runtime替代隔离验证。相关Rust lib与完整
`test_service_flow`均通过，真实Agine proof覆盖了本任务的canonical compile/assemble/publish
闭环。

## Actual write set

```text
artifact-identity/src/error.rs
artifact-identity/src/lib.rs
artifact-identity/src/package_artifact.rs
compiler/driver/generated_deployment.rs
compiler/driver/http_gateway_projection/mod.rs
compiler/driver/lib.rs
compiler/driver/websocket_gateway_projection.rs
compiler/tests/generated_service_deployment.rs
deployment/src/assembly/candidates.rs
deployment/src/assembly/error.rs
deployment/src/assembly/mod.rs
deployment/src/projection/error.rs
deployment/src/projection/mod.rs
deployment/src/projection/package_closure.rs
deployment/src/storage/package_copy_admission.rs
deployment/src/storage/records.rs
deployment/src/storage/tests.rs
test-runner/src/canonical_store.rs
test-runner/src/test_service_fixture.rs
test-runner/src/test_service_fixture/http_entry.rs
test-runner/tests/test_service_flow.rs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-K3-canonical-store-copy-admission-cache.md
  P5-F445H-I7-P8-K3-canonical-store-copy-admission-cache-result.md
```

未push。
