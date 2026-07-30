# P5-F339 Response error schema blocker reacceptance

状态：Completed（PASS）。结果见
`P5-F339-response-error-schema-reacceptance-result.md`。

## 直接父节点

- 原独立验收及唯一 blocker：
  `P5-F337-service-error-wire-checkpoint-acceptance-result.md`
- blocker 修复任务与结果：
  - `P5-F338-response-error-declarative-schema-fix.md`
  - `P5-F338-response-error-declarative-schema-fix-result.md`

本任务只复验 F337 B1 及其直接影响面。F337 已 PASS 的 fixed carrier、Rust exact bytes、telemetry
字段和 dependency 证据仍成立，除非确认 F338 实际改动触及或推翻这些代码事实。

## 精确候选

- F338 implementation commit：
  `3c69f12b9f81fe29827f3f5d43c489c6bee2cd22`
- F338 implementation tree：
  `7732c2b2920042712ce1d7a9b8b2aca32ed8ede7`
- integration merge commit：
  `79fccb88acdc8c85aafff3c88ea3d1b2532c46d0`
- integration merge tree：
  `7732c2b2920042712ce1d7a9b8b2aca32ed8ede7`
- 主 Agent 合流组合探针：
  - declarative shared header corpus；
  - existing header+payload shared corpus；
  - `2/2 PASS`，其余45个 selector跳过。

只读 production/tests/fixtures。唯一允许写入
`P5-F339-response-error-schema-reacceptance-result.md`并提交。不得修实现、测试、fixture、task或设计；
不得承接 H/R/T，不 push，不运行完整 Router/workspace/root/stable/live。

## 必须独立判断

1. `runtimeFrameHeaderSchemas['response.error']`的 production declarative schema现在是 exact
   discriminated union，而不是另一个 optional-property bag：
   - fixedService禁止 generic `error`和extra field；
   - control要求 generic `error`且 nested fields/required/status严格；
   - 两分支均固定 v2、response.error、非空 requestId。
2. schema表示的最小扩张不会改变其它 frame schema；production中需要读取 object properties的调用点
   完整处理 union，没有 `any`、宽泛 cast、遗漏 branch或运行时异常。
3. 测试真正解释 production schema；不是把 manual validator结果伪装成 schema结果，也不是只检查
   schema对象字段。
4. 同一份4正/30负 shared corpus被完整且互斥地分成 header-invalid 与 payload-only invalid：
   - 全部4个合法 header由 declarative schema接受；
   - 至少 `fixed-carries-generic-error`和`control-missing-error`由 declarative schema拒绝；
   - header-invalid全部拒绝；
   - payload-only case的 header仍接受、完整 frame仍由既有 seam拒绝；
   - corpus未来增删 case会让覆盖测试失败，而不是静默漏测。
5. interface、manual validator、declarative schema、header+payload seam语义一致；F338没有修改 wire
   interface、shared corpus、Rust、telemetry或 consumer。
6. 若以上 PASS，明确判断 F337 唯一 blocker关闭，C0可冻结并解除 H/R/T fan-out；这不代表
   H/R/T、W2-W、A6或 Phase 5完成。

## 独立证据

- 阅读 F338 exact diff、production schema、test evaluator及 shared corpus；
- 反搜 response.error declarative schema owner与 `ProtocolEnvelopeSchema`调用点；
- 先列出且确认 selector非零，只运行两条最小聚焦 selector：
  - declarative oneOf shared header corpus；
  - existing header+payload shared corpus；
- `git diff --check`；
- 核对 F338之外没有修改 F337已 PASS的 shared owner。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f339-response-error-reacceptance`
- branch：`codex/p5-f339-response-error-reacceptance`
- 由提出 F337 精确 blocker 的原验收 Agent复验，符合“同一 reviewer可复验自己刚提出的同一精确
  blocker”例外；
- 返回 PASS/FAIL、blocking、独立证据、C0/H-R-T gate判断与 result commit。
