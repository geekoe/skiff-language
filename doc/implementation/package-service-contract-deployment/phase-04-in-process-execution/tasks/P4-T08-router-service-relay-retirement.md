# P4-T08：Router Runtime-Service Relay Retirement

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2.6、§6.2、§12、§14、§15。
- 风险/验收组：高风险runtime/router boundary；与T07/T09合流后由R03验收。
- 当前成熟度：R02 runtime lanes PASS；完成后是router no-remote-service checkpoint。
- 有效证据：本任务clean commit及exact R02 checkpoint。runtime protocol service caller、forward registry/lifecycle、
  router tests或dependencies变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：R02 PASS；与T07/T09并行。
- 解锁：R03。
- branch：`codex/p4-t08-router-relay-retirement`。
- worktree：`/Users/geek/workspace/skiff-p4-t08-router`。
- 五分钟内真实edit。不要粗删同时服务gateway、actor/spawn的registry/control owner；先按caller kind和真实引用分类。

## 写入范围

独占`router/**`中runtime-originated service `request.start`、response/chunk/error/cancel forward生命周期与测试。
不得修改Rust runtime、tooling/registry authoring或T09 checker。

## 完成态

1. router收到runtime-originated `caller.kind=service` request start时，在runtime selection/lazy lookup/forward state
   创建前稳定拒绝；缺provider不解析build、版本或fallback caller build。
2.仅供service relay的pending forward response/stream/cancel owner和生产引用删除；不得留下另一消息类型绕过拒绝。
3.外部gateway ingress、runtime registration/health、actor/spawn及非service控制流保持原有语义和测试。
4. protocol错误清晰稳定，不泄漏registry内部；拒绝不污染pending map、runtime load或routing metrics。
5.没有RemoteBoundary placeholder、feature flag、compatibility allowlist或测试专用production exemption。

## 最早探针与唯一验证 ownership

```bash
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- --runInBand
```

若router test runner不接受参数，运行聚焦支持的精确命令并回报。必须覆盖service caller拒绝且lazy/runtime lookup计数
为零、stream/cancel无pending状态，以及gateway/actor/spawn回归。另运行`git diff --check`；不运行完整router gate。

## 回报

提交一个commit，回报retired call graph、保留control flow、protocol/负例、命令与自验收矩阵。
