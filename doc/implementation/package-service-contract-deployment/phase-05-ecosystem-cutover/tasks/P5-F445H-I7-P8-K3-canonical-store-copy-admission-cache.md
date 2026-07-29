# P5-F445H I7 P8 K3 canonical store copy admission cache

状态：

```text
IMPLEMENTED
```

## Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-K2-test-runner-pipe-output-flush-result.md`
- 相关fixture owner：
  `P5-F445H-I7-P8-K-test-runner-http-entry-closure-result.md`
- Skiff baseline：
  `90de5a2048f85c308068ec82e15b41966e4ea773`
  （tree `fb2f928b602e49e05047333d766397e7e8f95d0d`）
- integration owner：
  `/root/phase05_integration_steward`

真实Agine non-live证据显示，在case 1前复制同一canonical package closure时，
`validate_package_artifact_identities -> build_identity_from_projection -> canonical_json`
累计单核运行约12分钟。其它服务套件已经通过。

## Zero-worktree preflight

真实路径：

```text
CanonicalTestServiceFixture::publish
-> every case CanonicalTestRecords::publish
-> copy_package
-> source read_package_artifact
-> source resolve_package_artifact_schema
-> target write_package_artifact
```

同一个package record在每个case重复复制；单次复制内部还在read/ref、schema resolve和target
write间重复执行build projection identity验证。仅按`PackageArtifactRef`跳过后续复制会漏掉source
或target被篡改的情况。

首次实现后的真实 Agine publish-only probe 又确认了同一类重复校验的更早入口：

```text
assemble_test_service_fixture_inner
-> http_entry::project
-> generate_service_deployment
-> package_bindings
-> package_artifact_ref
-> validate_package_artifact_identities
-> build_identity_from_projection
```

并且同一 case 后续的 deployment projection 与 RuntimeAssembly candidate index 也会再次验证同一
package closure。因此本节点必须同时关闭 fixture assemble 与 fixture publish 两段重复；只缓存
`copy_package` 仍不足以在合理时间进入 case 1。

## Implementation boundary

最小安全闭环：

1. canonical store首次读取package + schema closure时完成完整identity/canonical验证，返回字段私有、
   不可Serde、不可外部构造的validated token。
2. token绑定record kind、canonical source root、exact package ref，以及每条record的exact bytes
   和length。exact bytes相等本身强于digest相等，命中路径不得为了重复证明同一事实而重新计算
   大型File IR的SHA-256。
3. 同进程cache仅存在于一次test-service fixture publish中；key包含source root与package ref，
   不持久化、不跨process、不跨source root。
4. cache hit仍从source安全读取每条record并做cheap length/exact-bytes核对；不同内容不得命中。
5. target从token写入exact bytes，继续走immutable exact-byte compare；首次admission不可跳过。
6. File IR和resource在首次admission时仍走现有完整内容/identity验证，并和package/schema
   record一起进入同一个exact-byte token；命中后只复核source exact bytes并向target做immutable
   exact-byte write，不能再次执行逐文件canonical identity投影。
7. fixture assemble 使用字段私有、不可Serde、持有 exact PackageArtifact 内容与 canonical bytes
   的进程内 admission；首次完整 identity validation 必须保留，后续 compiler/deployment/
   assembly 路径只能消费该 opaque admission，不能按 identity 字符串自行跳过。
8. 多 case 共享同一组 fixture package admissions；完整 package identity validation 次数只随
   unique package closure 大小变化，不随 case 数量变化。

## Expected write set and gates

```text
deployment/src/storage/mod.rs
deployment/src/storage/package_copy_admission.rs
deployment/src/storage/records.rs
deployment/src/storage/tests.rs
artifact-identity/src/package_artifact.rs
artifact-identity/src/lib.rs
compiler/driver/generated_deployment.rs
compiler/driver/http_gateway_projection/mod.rs
compiler/driver/websocket_gateway_projection.rs
deployment/src/projection/**
deployment/src/assembly/**
test-runner/src/canonical_store.rs
test-runner/src/test_service_fixture.rs
test-runner/src/test_service_fixture/http_entry.rs
test-runner/tests/test_service_flow.rs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-K3-canonical-store-copy-admission-cache.md
  P5-F445H-I7-P8-K3-canonical-store-copy-admission-cache-result.md
```

必须覆盖：

- same source root + same identity重复命中只产生一次完整admission；
- identity不同产生独立admission；
- source package/schema内容篡改后cache hit fail closed；
- source File IR/resource内容篡改后cache hit fail closed；
- target内容冲突仍fail closed；
- 首次invalid identity admission失败且不进入cache；
- 多case fixture assemble的完整package identity admission次数与case数量无关；
- focused deployment storage与test-runner定向测试；
- test-runner full、fmt、diff；
- 最终用Agine运行观察publish阶段能合理进入case 1，不运行其它stable/live/Mongo/OAuth/browser。

若opaque token无法保持上述边界，或性能根因不再是重复projection identity验证，则停止并报告。
