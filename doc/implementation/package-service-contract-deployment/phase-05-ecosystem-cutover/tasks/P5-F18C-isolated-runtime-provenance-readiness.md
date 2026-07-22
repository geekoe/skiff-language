# P5-F18C：Isolated Runtime Provenance and Readiness

权威设计：`doc/architecture/package-service-contract-deployment.md` §6.1、§6.2、§10、§11、§14；F15A/F16C与D20
result。从D20 docs checkpoint建立`/Users/geek/workspace/skiff-p5-f18c-isolated-boundary`、
`codex/p5-f18c-isolated-boundary`。全新Agent、一个commit，不merge/push/stable/Host；五分钟内修改。

exclusive write set：`scripts/lib/isolated-test-runtime-instance.mjs`、`scripts/lib/isolated-test-runtime.mjs`及
`scripts/tests/{isolated-test-runtime,test-runner-runtime-isolation}.test.mjs`。不改compiler/runner Rust、Router/Runtime、
supervisor、gate、manifest/lock。

完成态：一次计算absolute Cargo target，同值进入config/bootstrap/supervisor/test runner；hostile inherited
`SKIFF_TEST_PLATFORM_SOURCE_ROOT`必须由module-owned absolute skiff root覆盖，relative target不得由child cwd二次解释。
readiness只接受active exact tuple、`replica.connected === true/state===healthy/environment+generation+assembly exact`、
`capability.connected === true`且`runtimeId===replicaId`；missing字段一律false。

```bash
node --test scripts/tests/isolated-test-runtime.test.mjs scripts/tests/test-runner-runtime-isolation.test.mjs
node --check scripts/lib/isolated-test-runtime-instance.mjs
node --check scripts/lib/isolated-test-runtime.mjs
git diff --check
```

必须覆盖错ID、错/缺environment、connected缺失/false、错generation/assembly、分属不同runtime、hostile env与父子cwd
relative target。回报commit/tree/lock、exact env/config argv、cleanup、extra-review；不改变公开test CLI或readiness语义。
