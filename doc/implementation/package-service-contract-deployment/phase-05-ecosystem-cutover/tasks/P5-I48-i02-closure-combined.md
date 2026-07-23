# P5-I48：I02 Closure Combined

DAG节点I48，依赖F48A/B/C合流到commit
`ad847f7254521d1dd4679a4f8af72b2c88753310`、tree
`f0a33cc750025916df7b303e2f07b9db3f2e9c6d`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读owner各运行一次：

```bash
cargo test --locked -p skiff-test-runner package_service_contract_deployment
cargo test --locked -p skiff-runtime-eval spawn
cargo test --locked -p skiff-runtime-host spawn_submit -- --test-threads=1
node --test scripts/tests/package-service-i02-combined.test.mjs
git diff --check
```

必须闭合fixture effects/bindings、canonical execution projection与target、typed submitted receipt及transaction direct；
静态反搜无WS receipt泄漏、legacy program projection或unvalidated response。禁止编辑、提交、真实I02/R05/instance/stable/
full gate。PASS只解除I02C第三次且唯一完整combined。
