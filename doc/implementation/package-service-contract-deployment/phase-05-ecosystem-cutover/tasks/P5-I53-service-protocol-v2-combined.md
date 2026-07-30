# P5-I53：Service Protocol v2 Combined

DAG节点I53，依赖F52A/B/C/D与F53A–F全部合流到commit
`f8e71683bbd8002a94c68aa92cae2e82f834d554`、tree
`7575926f36725917e8f47ba8b8b41a862868c3f0`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读owner在同一候选上各运行一次：

```bash
cargo test --locked -p skiff-runtime-host runtime_config
cargo test --locked -p skiff-runtime-host register_mapper
cargo test --locked -p skiff-runtime-transport protocol
cargo test --locked -p skiff-runtime-loader
pnpm --dir router exec vitest run tests/protocol.test.ts tests/runtime-registry-dispatch.test.ts \
  tests/actor-spawn-runtime-control.test.ts tests/assembly-runtime-endpoint.test.ts \
  tests/router-default-spawn-probe.test.ts tests/manifest-validation.test.ts
pnpm --dir router type-check
node --test scripts/tests/package-service-dev-sync.test.mjs
git diff --check
```

静态分类必须证明：production/普通正例无`skiff-protocol-v1:sha256` SPI；只保留D53列出的刻意reject corpus；
`skiff-runtime-frame-v1`与`skiff-runtime-manifest-v1`保持，`skiff-runtime-manifest-v2`零命中；
`protocolVersion`从register生产/fixture消失。禁止编辑、提交、I02/R05/instance/stable/full gate。PASS解除I02F。
