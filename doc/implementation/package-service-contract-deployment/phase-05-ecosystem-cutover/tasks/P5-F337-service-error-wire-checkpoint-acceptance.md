# P5-F337 Service error wire checkpoint acceptance

状态：Completed（FAIL）。结果见
`P5-F337-service-error-wire-checkpoint-acceptance-result.md`。

## 验收输入

- 唯一权威设计：
  `doc/architecture/package-service-contract-deployment.md`，重点 6.3；
- current wire/observability owner：
  `P5-F333-wire-observability-delta-audit-result.md`；
- 被验收任务与结果：
  - `P5-F336-service-error-wire-telemetry-checkpoint.md`
  - `P5-F336-service-error-wire-telemetry-checkpoint-result.md`
- frozen fixed carrier与 diagnostic seam：
  - `P5-F332-service-error-channel-a5-acceptance-result.md`
  - `P5-F335-restricted-service-diagnostic-acceptance-result.md`

## 精确候选与边界

- 候选 commit：`fb29f806911b5dea3f1334e3d1af096248292897`
- 候选 tree：`eda84af5ba318f48c3a2b7b682b932c6855ce6c8`
- 合流探针：
  - Rust shared v2 corpus：1/1 PASS；
  - Router同一 corpus payload-byte probe：1/1 PASS。
- 风险：最高，shared wire/schema与跨语言 protocol parity。

只读 production/tests/fixtures。唯一允许写入
`P5-F337-service-error-wire-checkpoint-acceptance-result.md`并提交。不得修实现、fixture、task或设计；不得
运行完整 Router/telemetry/eval/workspace/root/stable/live，不 push，不承接 H/R/T。

## 必须独立判断

1. `FixedServiceResponseFailure`只有一个 owner；capability-context只 additive re-export，request-contract
   `ResponseEvent`直接复用，没有第二 carrier/envelope或 capability语义变化。
2. binary container与非 error frame仍为 v1；只有两种 `response.error`使用专用 v2。Rust和TS均无 v1
   response.error reader/writer、dual path或 fallback。
3. Rust header是 exact discriminated union；version/type/kind/requestId、control payload/status及
   fixed/control payload presence全部 fail closed。generic control不可能按 code/message/shape升级 fixed。
4. fixed encode/decode只调用 canonical `OpaqueServiceError` owner；public/Internal/platform都保留收到的
   exact bytes。Rust mapper、TS view和未来 Router forwarding不需要拆字段后重编码。
5. TS interface、declarative schema、manual validator与 header+payload seam表达同一 union；nested
   additionalProperties不漂移，strict view有限检查 envelope kind/platform identity/byte array/correlation。
6. 4正/30负 shared corpus确实由 Rust和TS直接读取；负例足以覆盖版本、variant混合、unknown/extra/missing、
   payload presence、malformed envelope、空白 identity/correlation与 control约束，且没有只在一端维护的
   等价 fixture。
7. Rust transport、Router TS和telemetry TS的 telemetry event都要求同一
   `visibility=operational|restricted`与 top-level `errorId`规则；restricted缺 trace/error id、
   unknown visibility/field失败。telemetry protocol版本保持既定 v1。
8. C0没有越界实现 host projection、gateway policy、storage/query/redaction；同时 shared API足以让
   H/R/T不新增兼容或第二 schema。result列出的断点与当前 production真实相符。
9. Cargo direct dependency方向只复用更低层 model且 lockfile变化精确；无循环依赖或额外 crate/API扩张。

## 独立证据

- 阅读 Rust request-contract/transport、Router protocol、telemetry protocol与共同 fixtures；
- 反搜旧 v1 error producer/reader只允许存在于待迁移 consumer或负例，shared owner本身不得命中；
- 不重复两条 shared-corpus合流探针；至少独立抽查：
  - Rust fixed mapper三种 envelope byte round trip；
  - matching Internal code/message仍为 generic control；
  - Rust或telemetry TS的一组 visibility/restricted correlation正负例；
- selector先列出且非零，只运行最小聚焦命令；
- 核对 F336 implementation之后受验 shared production/fixture没有额外变化。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f337-error-wire-acceptance`
- branch：`codex/p5-f337-error-wire-acceptance`
- 新的一次性独立验收 Agent；
- 返回 PASS/FAIL、blocking、non-blocking、独立证据、consumer断点判断与 result commit；
- PASS只冻结 C0并解除 H/R/T fan-out，不代表 W2-W/A6/Phase 5。
