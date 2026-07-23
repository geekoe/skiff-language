# P5-I35：Actor Control / I02 Combined

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§7、§10、§12及§14。

DAG节点I35，依赖F45A–F45E全部合流到production commit
`dada6d56a42d5eb917ec96db200fc2567b8195df`、tree
`ccd7445a59455fde24f17d71260d473bd208a658`、Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。它是新shared wire/consumers与I02 harness合流后的唯一cheap
combined owner，不作R05C/I02/R02 verdict。

全新只读Agent在exact候选各运行一次：

```bash
cargo test --locked -p skiff-runtime-capability-context -p skiff-runtime-transport
cargo test --locked -p skiff-runtime-host host::router_session::tests:: -- --test-threads=1
pnpm --filter @skiff/router exec vitest run \
  tests/protocol.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/actor-spawn-runtime-control.test.ts
pnpm --filter @skiff/router type-check
node --test scripts/tests/package-service-i02-combined.test.mjs
node scripts/skiff.mjs test test-runner/fixtures/package-service-i02-spawn-submit
git diff --check
```

必须确认：

- shared corpus、Runtime current-context、Router active/pinned-draining授权及structured queue roundtrip；
- new fixture从normal source真实compile/test，不使用std.actor、manual emitter或legacy runtime.register；
- I02 transaction direct仍覆盖withdrawal/tamper/reject/rollback；
- D46 claim worker保持fail closed且未被本combined冒充执行成功。

禁止编辑、提交、修复、真实R05/I02、instance/stable或完整gate。FAIL返回精确层级与唯一owner，不重跑。PASS只解除
全新R05C owner在新wire candidate上重建一次generation lifecycle证据；R05C PASS后才可运行I02。

shared wire/consumer、Router lifecycle、Runtime host、fixture/scripts、Cargo.lock或checkout source变化使I35失效。
