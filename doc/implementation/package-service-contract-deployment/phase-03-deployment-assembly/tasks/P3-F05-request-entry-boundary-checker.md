# P3-F05：Request-entry Boundary Checker Coverage

## 权威输入、失败与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§9、§10、§12、§14。
- 执行输入：P3-A01 对 production candidate `34b6a863534b435d1e81b88de4cd8c0ed8a352fa` 的
  blocking finding A01-11：`whole-assembly-host` subject未包含真实
  `runtime/host/src/host/request_entry.rs`，canonical anchor discovery也无法发现它；现有 lazy-load mutation只写入
  admission owner。
- 风险/验收组：中风险 structure-gate repair；不修改 Rust production代码。
- 有效证据状态：旧 boundary checker、自检与 runtime gate内嵌结构证据失效；其它 runtime动态测试保持有效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent合流后统一重建受影响证据。

## DAG 与执行约束

- 依赖：A01初次验收 FAIL；可与 F04并行。
- 解锁：T09R affected-gate rebuild。
- branch：`codex/p3-f05-request-entry-checker`。
- worktree：`/Users/geek/workspace/skiff-p3-f05-request-entry-checker`。
- 先补精确 subject与真实路径 mutation，不扩大为路径通配 allowlist或 known-violation ledger。

## 写入范围与完成态

- 只修改 `scripts/check-runtime-artifact-boundaries.mjs`、
  `scripts/lib/runtime-artifact-boundary-{checker,subjects,self-test}.mjs` 及直接测试；不得修改 `runtime/**` Rust
  production、Cargo或其它 checker。
- 将 `runtime/host/src/host/request_entry.rs` 纳入明确 production owner；若采用 discovery，必须有不依赖当前
  canonical anchor文本且能稳定覆盖 terminal request consumer的精确规则。
- self-test必须在真实 request-entry subject路径注入 request-time lazy-load变异，并证明 production checker失败；
  不能只在 admission owner或 synthetic fixture验证。
- subject-omission自测必须覆盖删除/遗漏 request entry；production checker对 clean tree仍PASS。
- 不豁免旧 DTO、raw JSON/display/source linking、fallback/dual-read等既有 DENY规则。

## 唯一验证 ownership

```bash
node scripts/check-runtime-artifact-boundaries.mjs --self-test
node scripts/check-runtime-artifact-boundaries.mjs
git diff --check
```

额外给出 selector probe，证明 `request_entry.rs` 出现在最终 subject集合；不得运行完整 runtime selector。

## 回报

提交一个 commit，回报 commit、subject索引、真实 request-entry mutation与 omission负例、全部自测矩阵和命令结果。

