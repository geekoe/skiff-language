# P5-I27：Fixture Pipeline Cheap Combined

权威设计为
`doc/architecture/package-service-contract-deployment.md`：

- §2“不变量”第2、4、8、9条；
- §3“Package 与 PackageArtifact”中PackageArtifact是compile产出的typed闭包且identity由规范内容决定的条款；
- §5“ServiceDeployment”中typed implementation/operation/ingress/dependency binding及完整闭包校验条款；
- §12“RuntimeAssembly 与扩容”中完整deployment/package闭包、active assembly与replica可观测条款；
- §13“Registry、Release 与 Publish”中不可变records先于可更新pointer的publish语义；
- §14“Fail-closed 条件”中dependency、version或identity不匹配必须在请求前失败的条款。

DAG节点为I27，依赖F27A、F27B、F27C全部合流；验证其共享接口后只解除R29。当前共享接口是compiler
`PublishedPackageArtifactReceipt`及唯一records writer、test-runner `CanonicalStdSeedReceipt`及CAS pointer、smoke strict
receipt/readiness oracle。风险等级高，验收分组为F23D fixture pipeline combined。进入状态是上述实现已合流的exact clean
integration candidate；PASS后成熟度从Implementation Checkpoint提升为Risk-Accepted Candidate，但不完成F23D或Phase 5。
冻结的production candidate为commit `3987923cb9abc5c852a4d8d9d16d347c5873138f`、tree
`f7457b1d11a43406763184e8ff220277d6ac6049`、Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`；随后只允许本I27/R29合同文档提交，派发时另给包含合同的exact HEAD。

全新只读integration Agent执行。禁止编辑、提交、修复、启动Router/runtime/activation或访问stable；不运行top-level
smoke、full、I16或Host gate。各命令在同一candidate、仓库根目录、默认环境中至多执行一次，按序fail-fast：

```bash
cargo test --locked -p skiff-compiler --lib authoring:: -- --test-threads=1
cargo test --locked -p skiff-test-runner --lib canonical_std_seed -- --test-threads=1
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment ecosystem_fixture_has_no_artifact_rewrite_or_synthetic_stream_bridge -- --exact --test-threads=1
cargo test --locked -p skiff-test-runner --test canonical_std_seed_bootstrap -- --test-threads=1
node --test scripts/tests/package-service-ecosystem-smoke-real.test.mjs scripts/tests/package-service-ecosystem-smoke-diagnostic.test.mjs scripts/tests/isolated-test-runtime.test.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-runtime-crate-dag.mjs
pnpm --filter @skiff/router type-check
git diff --check
git status --short
```

第四条integration test就是唯一不启服务的真实fixture Cargo bootstrap：它必须执行`--bootstrap-only --locked`顺序并验证typed
receipt、immutable records与pointer；不得另跑一次手工Cargo命令。每组测试数必须非零。`git status --short`只允许integration
基线已记录的`.p5-i16-combined-ledger.json`，不得出现其它变化。任一失败立即给I27 FAIL，不修复、不重跑；全部通过才给I27
PASS并解除R29。

证据仅对任务文件记录的exact commit/tree/Cargo.lock blob有效。compiler、test-runner、smoke scripts/fixtures、Router、
runtime DAG、platform source、Cargo.lock或本合同命令发生变化都会使I27失效。
