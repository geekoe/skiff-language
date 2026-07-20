# P5-T05：Test Runner / Package Test / Fixture Cutover

## 权威输入与DAG

- 设计：`doc/architecture/package-service-contract-deployment.md` §1–§6、§9–§12、§14–§15。
- 依赖：R01 PASS exact checkpoint；与T02–T04同级，解锁R02。
- 风险：中高；test runner是真实production tool，fixture不得掩盖旧执行owner。
- branch：`codex/p5-t05-test-infrastructure`。worktree：`/Users/geek/workspace/skiff-p5-t05-tests`。
- 当前共享状态是R01 PASS的implementation checkpoint；完成后仍只是R02 batch candidate。使用新的开发
  Agent；证据对T01 artifact interfaces、owned harness/tests/fixtures、Cargo或smoke script变化失效。
- 五分钟内产生code edit；无法从T01构造canonical isolated assembly时报精确缺口。

## 写入范围

独占 `test-runner/**`、`runtime/package-test/**`、test-only canonical artifact/assembly fixture builders、
isolated test runtime harness与直接tests。大文件`service_publish.rs`、`package.rs`、`runtime_process.rs`
不继续增加新owner；拆出canonical build/store/assembly模块。不改runtime host production、router、
compiler/scripts production、artifact-model exports或verify接线。

## 完成态

1. package tests从Package source直接编译PackageArtifact，dependency只解析canonical
   PackageArtifact/ServiceContract；不写或读PackageUnit。
2. service/ingress execution tests使用code-free contract + implementation PackageArtifact + deployment +
   RuntimeAssembly的isolated immutable store，不构造synthetic ServiceUnit/serviceAssembly。
3. package-test direct calls仍保持same-heap semantics；需要service call的test通过canonical binding/
   InProcessBoundary，不把两者合并。
4. test-only source overlay不改写production PackageArtifact identity/path；fixture中的config/state/double有明确
   deployment/test owner。
5. 旧service publish/runtime pointer writer/artifact graph helper删除或被canonical replacement完整替代；
   每组删除的旧测试说明replacement或语义整体退役。
6. non-live isolated tests不写stable root、不调stable reload，且实际从指定Skiff checkout运行。
7. 提供 `scripts/run-package-service-ecosystem-smoke.mjs --replicas 2` 的隔离动态入口，使用两个
   runtime-home加载同一assembly并断言Host request最终结果、replica failover与failed reload rollback。

## 探针与唯一聚焦验证 owner

- package-only、contract-dependent package、provider/consumer service、ingress、package-direct mutation各一个真实正例。
- missing/tampered artifact、ambiguous provider、test overlay identity rewrite、service call without deployment负例。
- output tree反向搜索无PackageUnit/ServiceUnit/serviceAssembly/index/pointer。

```bash
cargo test -p skiff-test-runner --test package_service_contract_deployment
cargo test -p skiff-runtime-package-test --test package_artifact
node scripts/run-package-service-ecosystem-smoke.mjs --replicas 2 --self-test
git diff --check
```

不跑完整tests/verify/live。提交一个commit并合入Skiff integration branch，回报fixture data flow、旧测试disposition、反向
搜索及自验收矩阵。
