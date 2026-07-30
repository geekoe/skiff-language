# P5-I51：Router Timeout Diagnostic Combined

DAG节点I51，依赖F51A/B合流到commit `e3b93c4ef6907d59e3a58e7ab17448ccec34c4d0`、tree
`7448c83a8e322f7631269a9111518ecb0ba88f30`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读owner各运行一次：

```bash
pnpm --dir router exec vitest run tests/router-default-spawn-probe.test.ts
pnpm --dir router type-check
node --test scripts/tests/isolated-test-runtime-log-evidence.test.mjs scripts/tests/isolated-test-runtime.test.mjs
node --test scripts/tests/package-service-i02-combined.test.mjs
git diff --check
```

必须确认默认Router submit正/负终止、失败日志bytes/hash/bounded redaction、cleanup后错误证据仍可序列化，
以及I02 direct transaction未漂移。静态检查错误对象到outer smoke ledger的传播不丢失enumerable evidence。
禁止编辑、提交、真实I02/R05/instance/stable/full gate。PASS只解除I02E。
