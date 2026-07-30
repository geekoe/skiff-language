# P5-I53A：Service Protocol v2 Reacceptance

依赖I53 FAIL及F53H/I、F54A/B合流到commit
`ee21b85ddd70c63585af6961ce4ea1ef8d4ec37e`、tree
`e67a9f23f43b23a26b1915230fa592935f55b7d2`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。I53其余PASS证据继续有效。

全新只读owner各运行一次：

```bash
cargo test --locked -p skiff-runtime-host runtime_config
cargo test --locked -p skiff-runtime-loader
cargo test --locked -p skiff-artifact-identity service_protocol_identity_hash
pnpm --dir router exec vitest run tests/test-dispatch.test.ts tests/artifact-reload.test.ts
pnpm --dir router type-check
git diff --check
```

静态重做I53精确允许清单：普通SPI正例/production无v1；只保留刻意reject；frame-v1与manifest-v1保持；
manifest-v2零命中；register `protocolVersion`只剩absence/reject证据。禁止编辑、提交、I02/R05/gate。PASS与I53
既有证据合并后解除I02F。
