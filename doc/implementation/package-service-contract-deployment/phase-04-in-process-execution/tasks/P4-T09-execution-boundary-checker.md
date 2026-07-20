# P4-T09：Runtime Execution Boundary Checker

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §6、§7、§9、§12、§14、§15。
- 风险/验收组：中高风险structure gate；与T07/T08合流后由R03验收，T10重建final evidence。
- 当前成熟度：R02 lane checkpoint；完成后提供checker engine与hermetic mutation coverage，production零违规由T09R负责。
- 有效证据：本任务commit及exact subject registry/checker state。production文件移动/改名、owner registry、规则、
  self-test fixture或被扫代码变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：R02 PASS；可与T07/T08并行。
- 解锁：T09R；T07/T08/T09合流后才运行production registration/check。
- branch：`codex/p4-t09-execution-boundary-checker`。
- worktree：`/Users/geek/workspace/skiff-p4-t09-checker`。
- 五分钟内真实edit；checker不能以当前文件名硬编码通过而漏扫移动/重复owner。

## 写入范围

独占`scripts/check-runtime-execution-boundaries.mjs`、`scripts/lib/runtime-execution-boundary-*.mjs`、hermetic self-test
fixtures与脚本测试。不得修改Rust/TypeScript production；真实subject registration与verify接线留给T09R。

## 完成态

checker engine和self-test必须能够枚举/证明：

1. canonical ingress和internal call只有一个`InProcessBoundary` dispatcher owner；不存在第二dispatcher、remote
   selection/fallback或assembly→legacy outbound call edge。
2. ActivationContext/request generation/callback table不在shared code owner或PackageBuildId cache；callback carrier
   不含method table/native address且recoverable production encoder拒绝。
3.执行user code的async/stream `tokio::spawn`只经owned context carrier；production无current-service TLS/task-local。
4. host request entry是required exact owner，只查active assembly，不lazy-load artifact/旧route fallback。
5. router明确拒绝runtime-originated service relay，service caller不进入runtime registry/lazy/forward owner。
6. test-only cfg识别精确，不能用文件名或测试helper掩盖production违规。

self-test至少覆盖违规注入、owner改名/移动/重复、required subject/file/registry omission、test-only伪例外、第二
dispatcher、TLS、shared callback table、recoverable callback、host fallback与router relay。每个mutation命中稳定ID。

本任务独立branch上T07/T08尚未合流，production checker允许精确报告已知host fallback/router relay违规；不得通过
allowlist伪报零。T09R才负责在merged production state注册exact owner并要求零违规。

## 唯一验证 ownership

```bash
node scripts/check-runtime-execution-boundaries.mjs --self-test
node --test scripts/tests/runtime-execution-boundary-checker.test.mjs
git diff --check
```

## 回报

提交一个commit，回报proposed subject/required-owner表、mutation matrix、已知pre-merge production findings、命令与
自验收矩阵；不得声称production violations=0。
