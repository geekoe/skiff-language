# P5-I49：Recoverable Owner Combined

DAG节点I49，依赖F49合流到commit `42f322364f46f0be9350f4535ff492a562e73ae1`、tree
`9692c132cd07b06a1935772d63deea1ec86467c3`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读owner各运行一次：

```bash
cargo test --locked -p skiff-runtime-eval recoverable
cargo test --locked -p skiff-runtime-eval spawn
git diff --check
```

必须确认duplicate packageId/different build的plain-data canonical hook construction成功，实际package-owned
LocalConcrete仍按packageId歧义fail closed；canonical spawn继续消费admitted execution projection并提交exact
target。静态反搜确认无assembly-wide eager检查、无PackageBuildId/version/slot/artifact durable key、无first-win/
compat/dual path。禁止编辑、提交、真实I02/R05/instance/stable/full gate。PASS只解除下一次唯一完整I02D。
