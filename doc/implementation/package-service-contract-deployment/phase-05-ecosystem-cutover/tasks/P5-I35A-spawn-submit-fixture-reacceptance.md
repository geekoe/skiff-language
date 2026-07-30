# P5-I35A：Spawn Submit Fixture Reacceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§7、§10、§12及§14。

DAG节点I35A，只复验I35唯一contract invocation blocker。exact production candidate保持
`dada6d56a42d5eb917ec96db200fc2567b8195df`、tree
`ccd7445a59455fde24f17d71260d473bd208a658`、Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`；I35其余PASS证据仍有效。

全新只读Agent新建唯一hermetic临时目录并运行一次：

```bash
P5_I35_ARTIFACT_ROOT="$(mktemp -d /tmp/skiff-p5-i35-artifacts.XXXXXX)"
node scripts/skiff.mjs test \
  test-runner/fixtures/package-service-i02-spawn-submit \
  --artifact-root "$P5_I35_ARTIFACT_ROOT"
git diff --check
```

artifact root必须为空、非stable且在结束后清理复核。禁止重复I35其它命令、编辑、提交、真实R05/I02、instance/stable或
完整gate。第一行`I35A PASS`或`I35A FAIL`；FAIL给精确compile/test或环境owner，不重试。PASS与I35其余证据共同解除
R05C。
