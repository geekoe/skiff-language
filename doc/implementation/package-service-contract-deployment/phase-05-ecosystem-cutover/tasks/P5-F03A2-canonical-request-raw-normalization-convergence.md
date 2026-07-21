# P5-F03A2：Canonical Request Raw / Normalization Convergence

## 输入、owner与限制

- 依赖：P5-D06完整矩阵；输入为R02A第二次FAIL exact candidate `571549739239ca16b04d09cd7be1716125dc1982`。
- 返回原F03A shared-seam owner的新repair worktree/branch；五分钟内产生code edit，只提交一个repair commit，
  不merge/push。
- 独占canonical request的`router/src/protocol/**`、`runtime/transport/src/runtime_assembly_request/**`、
  `runtime/transport/Cargo.toml`、直接exports/tests及
  `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`与request self-test接线。
- root `Cargo.lock`提交前恢复；不改activation/store、compiler/deployment、Router gateway/endpoint、Runtime
  host/driver、test-runner或四对象schema。

## 完成态

1. TS/Rust raw decoder对D06 Unicode scalar、decoded duplicate-key、UTF-8/control与number矩阵拥有同一接受集合；
   unsafe opaque integer在精度丢失前拒绝，合法opaque value得到同一canonical typed value。
2. Rust number归一复用`skiff-canonical-json`的唯一leaf owner并补safe-integer domain校验；TS production decoder
   对同一validated value做等价归一。不得在fixture或consumer增加修正规则。
3. canonical TS/Rust decoded header统一materialize HTTP/WS `adapterArgs=[]`、
   `testEffectsEnabled=false`、`testEffectDoubles={}`；四组absent/default pair必须比较完整decoded value，
   canonical TS type不得再声明运行时不存在的required字段或把已materialize字段留optional。
4. TS canonical binary入口不再先scan再用另一parser重解同一header；binary framing、strict JSON decode与metadata
   validation各有单一职责。legacy binary/request入口的接受集合与返回形态不变。
5. shared corpus直接进入两端production binary decoder，至少包含D06冻结的raw cases；accepted、rejected、
   normalized、legacy计数由同一self-test核对，合法surrogate与finite fractional positive seed roundtrip。

## 验证与回报

```bash
cargo test -p skiff-runtime-transport runtime_assembly_request_start
pnpm --filter @skiff/router type-check
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
git diff --check
```

另反搜canonical入口无scanner + second JSON parser、无fixture validator、无legacy alias放宽。回报raw case与四组
default pair的TS/Rust decoded结果、exact source/commit/tree、单commit/clean/lock状态。合流后主integration owner
只运行request combined probe；通过后原R02A reviewer才执行第三次且只限shared request的正式verdict。
