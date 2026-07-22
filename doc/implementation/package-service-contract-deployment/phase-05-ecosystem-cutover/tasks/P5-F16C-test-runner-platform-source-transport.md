# P5-F16C：Test Runner Platform Source Transport

## 输入与DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；阶段标准6；D18/F16A合同。
- 输入：F16A exact checkpoint；从其合流commit建立
  `/Users/geek/workspace/skiff-p5-f16c-runner-platform-transport`、分支
  `codex/p5-f16c-runner-platform-transport`。与F16B并行，禁止修改其owner。
- 高风险test production consumer；一个clean commit，不merge/push、不改stable。完成后只解除I16的F16C分支
  前置；I16仍须等待F16B、F17合流且无在途写入。
- 使用新的开发Agent，不复用F16A、D18 auditor或文档reviewer。
- 五分钟内开始实际修改；若F16A API不能在本owner内直接消费，立即报`TASK_NOT_EXECUTABLE`，不得回改shared owner。
- 证据只对F16A exact API、runner/fixture/source-suite/isolated caller、platform source内容、Cargo.lock与本任务commit
  不变时有效。

## 写入owner与完成态

owner限`test-runner`的CLI/options/canonical package与smoke fixture compile transport，
`scripts/lib/skiff-source-test-suite.mjs`、`scripts/skiff.mjs`的test入口、
`scripts/lib/isolated-test-runtime-instance.mjs` bootstrap、`scripts/lib/verify-live-plan.mjs` runtime-live argv及直接
tests。另只对
`scripts/lib/encrypted-storage-live-harness.mjs`和其test增加同一platform-root transport；该harness其余四对象/CLI
迁移仍归T06，禁止顺手修改。新增小型`verify-live-plan-platform-source.test.mjs`，不得扩张1158行`verify.test.mjs`。
还独占test-only gate harness `scripts/run-platform-source-shared-target-probe.mjs`、其command-double test及merge-only
`scripts/tests/platform-source-transport-combined.test.mjs`。消费F16A唯一context，不创建第二platform-root resolver。

- runner和`skiff-package-service-smoke-fixture`严格接收一次内部`--platform-source-root <absolute-root>`；所有
  canonical package/overlay/fixture compile使用同一context，missing/duplicate/relative/invalid root在编译前失败。
- `skiff test`、canonical source-suite的std entries、Host fixture preparer与Host consumer runner都从模块确定的
  absolute `skiffRoot`传同一参数；改变cwd或共享target不改变它。
- official std fallback只能通过F16A context匹配canonical manifest；普通root复制reserved manifest继续拒绝。
- integration test提供固定过滤名`platform_source_context_contract`覆盖runner负例，并提供test-only
  ignored `platform_source_identity_probe`：只在该Rust integration-test target从
  `SKIFF_TEST_PLATFORM_SOURCE_ROOT`读取probe root，打印带标签的exact prelude identity与std PackageBuildId供I16比较；
  production Rust/JS/binary不得读取该环境变量。这是`SKIFF_TEST_*` platform-root禁令的唯一test-only例外。
- std identity必须exact等于
  `skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4`。
- gate harness提供严格分离且必选的`--mode combined|full`：`combined`只实现candidate/space/worktree/shared-target/
  identity/structure/Fresh/cleanup矩阵，生成可复用JSON ledger；`full`只在核对同一candidate的combined ledger后执行一次
  A-origin build→B-root Fresh→原样Host完整gate，不重复combined/local矩阵。command-double tests覆盖两个mode且不启动真实
  build。merge-only combined fixture跨F16B/F16C检查所有production argv共享同一absolute root及一个omitted-root直接失败
  路径；F16C分支不运行任一真实mode，只有exact合流后的I16运行`combined`，G16在R16 PASS后运行`full`。
- 不改变公开`skiff test`参数语义、source registry、test count、activation/readiness/request路径或Host receipt。

不改F16A shared context/prelude、compiler binary/authoring JS、Router/Runtime、fixture业务语义、manifest/lock。
直接触碰大文件需extra-review；除上述单一ignored integration probe外，不得把platform root放入ambient
`SKIFF_TEST_*`环境变量。`verify-live-plan.mjs`只允许surgical argv接线，不新增职责；T06后续迁移必须保留该transport。

## 唯一聚焦验证

```bash
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment
node --test scripts/tests/skiff-source-test-suite.test.mjs scripts/tests/skiff-test-cli.test.mjs scripts/tests/test-runner-runtime-isolation.test.mjs
node --test scripts/tests/encrypted-storage-live-harness.test.mjs
node --test scripts/tests/isolated-test-runtime.test.mjs scripts/tests/verify-live-plan-platform-source.test.mjs scripts/tests/platform-source-shared-target-probe.test.mjs
cargo fmt --all -- --check
git diff --check
```

tests覆盖runner/fixture/compiler context参数全链、cwd变化、fake reserved root、missing/duplicate/relative、runtime-live/
encrypted/bootstrap caller、两个gate mode的命令编排、combined-ledger/candidate拒绝、cleanup refusal及source registry不变。
不得运行merge-only combined fixture、任一真实gate mode、原样source-suite、Host或完整verify。回报commit/tree/lock blob、
所有production caller反向搜索、文件行数与extra-review自验收矩阵。
