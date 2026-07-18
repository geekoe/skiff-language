# P2-T05B：Terminal Compiler Structure Gates

状态：active split；从 package-only dataflow checkpoint `735c8f1` 后的 terminal integration 开发，
与 T05 driver/facade 和 T05A projection/emission 并行。

## 目标与 ownership

- 独占 compiler boundary、crate DAG、rustdoc public API policy/checker 及其 self-tests/fixtures。
- 不修改任何 Rust production/compiler test 代码，也不修改 runtime/router/test-runner。
- checker 必须把终态两条 producer 作为唯一合法 compiler public shape：package compile 与 code-free
  contract compile。

## 完成态

1. production compiler 重新引入 `PublicationInput`、`PublicationKind`、`CompiledPublication`、
   `LoweredPublication`、PackageUnit/ServiceUnit/serviceAssembly producer、service publication facade、
   legacy/compatibility adapter 或 provider inference 时，结构 gate fail closed。
2. checker 不使用“暂时允许旧路径”的 allowlist；T05/T05A 中间态导致的 production failure可以记录，
   但 self-test/negative fixtures 必须独立通过。
3. crate DAG/public API policy 只声明 terminal owners，不为已删除 crate edge 保留例外。

## 验证

- 每个修改 checker 的 self-test/fixture。
- `git diff --check` 与相关 Node type/syntax check。
- production checker 在合流前允许因 T05/T05A 尚未完成而 FAIL，但必须给出预期命中；合流后由 T07
  运行最终 PASS。

提交并保持 worktree clean；回报 commit、禁止项矩阵、self-test 与合流前预期 production failure。
