# P5-F445H I7 P8 R Router ordinary ingress reuse

状态：

```text
READY_FOR_ZERO_WORKTREE_PREFLIGHT
EXPECTED_RESULT = NO_PRODUCTION_CHANGE
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-D0-http-entry-test-authority-result.md`
- baseline：
  `3a87d37f81a04c249f308b311bd91dcfdf3a8aa3`
  （tree `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`）
- DAG：`D0 -> R -> T`
- K/H禁止重叠；integration owner：`/root/phase05_integration_steward`

## 2. Goal

R证明self-ingress可以完全复用普通Router business HTTP：

- `x-skiff-service + x-skiff-version + method + path`命中精确case deployment；
- Host只形成HTTP request metadata，不参与路由；
- unary与raw HTTP server stream均走现有dispatch/backpressure/disconnect路径；
- Router不需要知道test case、inline effect或父finalization。

先审计现有`service-deployment-selection`、`assemblyHttpGateway`及相关测试。已有证据完整时提交
`NO_PRODUCTION_CHANGE` result；缺失证据时只补最小Router测试。

允许写集：

```text
router/tests/**
doc/.../P5-F445H-I7-P8-R-*-result.md
```

默认禁止修改`router/src/**`。只有现有普通路由真实RED且不改变权威语义时才上报主Agent重排，不能自行修。

## 3. Evidence and negatives

聚焦证据：

```text
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- <smallest relevant selectors>
git diff --check
```

覆盖selector缺失/错误version/错误service/错误method/path、相同Host不同service正常区分、stream client
disconnect。任何test-only route/header/session/token或Host route建议都是blocking scope expansion。
