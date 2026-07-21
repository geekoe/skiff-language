# P5-F16C：Test Runner Platform Source Transport

## 输入与DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；阶段标准6；D18/F16A合同。
- 输入：F16A exact checkpoint；从其合流commit建立
  `/Users/geek/workspace/skiff-p5-f16c-runner-platform-transport`、分支
  `codex/p5-f16c-runner-platform-transport`。与F16B并行，禁止修改其owner。
- 高风险test production consumer；一个clean commit，不merge/push、不改stable。完成后与F16B共同解除I16。

## 写入owner与完成态

owner限`test-runner`的CLI/options/canonical package与smoke fixture compile transport，
`scripts/lib/skiff-source-test-suite.mjs`、`scripts/skiff.mjs`的test入口、必要isolated bootstrap及直接tests。
消费F16A唯一context，不创建第二platform-root resolver。

- runner和`skiff-package-service-smoke-fixture`严格接收一次内部`--platform-source-root <absolute-root>`；所有
  canonical package/overlay/fixture compile使用同一context，missing/duplicate/relative/invalid root在编译前失败。
- `skiff test`、canonical source-suite的std entries、Host fixture preparer与Host consumer runner都从模块确定的
  absolute `skiffRoot`传同一参数；改变cwd或共享target不改变它。
- official std fallback只能通过F16A context匹配canonical manifest；普通root复制reserved manifest继续拒绝。
- 不改变公开`skiff test`参数语义、source registry、test count、activation/readiness/request路径或Host receipt。

不改F16A shared context/prelude、compiler binary/authoring JS、Router/Runtime、fixture业务语义、manifest/lock。
直接触碰大文件需extra-review；不得把platform root塞入ambient `SKIFF_TEST_*`环境变量。

## 唯一聚焦验证

```bash
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment
node --test scripts/tests/skiff-source-test-suite.test.mjs scripts/tests/skiff-test-cli.test.mjs scripts/tests/test-runner-runtime-isolation.test.mjs
cargo fmt --all -- --check
git diff --check
```

tests覆盖runner/fixture/compiler context参数全链、cwd变化、fake reserved root、missing/duplicate/relative及source registry
不变。不得运行原样source-suite、Host或完整verify。回报commit/tree/lock blob、所有production caller反向搜索、文件
行数与extra-review自验收矩阵。
