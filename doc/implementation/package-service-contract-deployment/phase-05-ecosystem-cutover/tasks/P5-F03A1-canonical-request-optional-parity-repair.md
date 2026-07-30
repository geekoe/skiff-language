# P5-F03A1：Canonical Request Optional-field Parity Repair

## 输入、owner与限制

- 依赖：P5-D03完整字段矩阵；输入为R02A FAIL candidate `a7566bb`。
- 使用原F03A seam owner的新repair worktree/branch；五分钟内产生code edit，只提交一个repair commit，不merge/push。
- 独占`router/src/protocol/**`、`runtime/transport/src/runtime_assembly_request/**`及直接exports/tests、
  `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`与verify request-parity接线。
- 不改activation frame、compiler/deployment store、Router endpoint/gateway、Runtime host/driver、test-runner或root lock。

## 完成态

1. D03列出的canonical request top-level optional fields及nested closure在TS/Rust拥有相同接受集合；unknown、
   wrong type、identity/pattern、integer/range、enum、array-item与跨字段mutation双端fail closed。
2. 所有production规则仍只有TS/Rust各一个decoder；shared corpus只保存值与mutation，不成为第三套validator。
   reusable strict-object/value helpers按职责拆分，不继续扩大`runtimeProtocol.ts`。
3. legacy request接受集合不因本repair改变；canonical字段不能通过legacy top-level alias或宽松fallback进入。
4. F03B/F03C需要的HTTP、server-stream与WS metadata保留；consumer只消费冻结后的typed header，无需再次改wire。

## 验证与回报

```bash
cargo test -p skiff-runtime-transport runtime_assembly_request_start
pnpm --filter @skiff/router type-check
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
git diff --check
```

另运行D03矩阵要求的cross-language mutation计数与legacy non-regression direct tests。回报字段组→owner→mutation→
两端结果、exact source/commit/tree、单commit/clean/lock状态。R02A原reviewer只窄复验该repair与失效证据。
