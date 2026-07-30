# P5-F278 Same-heap semantic correction result

状态：`PASS`；真实 identity observation 已与 alias、mutation、escape 和 unknown effect 分离。

## Exact candidate

- implementation commit：
  `1b31744d`
- integration merge commit：
  `9acc588c`
- 直接父任务：
  `P5-F278-same-heap-semantic-correction.md`

## 冻结语义

- 只有 caller-reachable 引用的 `==` / `!=` 或明确 identity intrinsic 产生
  `requiresSameHeapIdentity`。
- Map/JsonObject get 继续保留精确 caller projection 与 `returnsCallerAlias`，但不产生 identity 位。
- mutation、field store、interface boxing、ordinary throw/rethrow 和 unknown target 不再伪造
  identity observation；它们继续由各自的 write、escape、throw 或 unknown fact 管理。
- 已发生的真实 identity observation 仍无条件使 boundary unavailable，不能被 fresh/detached fact 抵消。
- `throw_origins`、`throwsCallerAlias` 与 `detached_error` 仍是独立的 error provenance /
  boundary safety 事实，开放错误通道不得删除或改写它们。

本结果只冻结 same-heap 与 provenance 的职责边界，不定义错误类型集合、错误 wire 或 runtime exception。

