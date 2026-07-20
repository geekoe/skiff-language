# P4-T09R：Merged Production Registration / Check

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §6、§7、§9、§12、§14、§15。
- 风险/验收组：中高风险structure gate finalization；完成后由R03验收。
- 当前成熟度：T07/T08/T09已合流的pre-entry-acceptance candidate。
- 有效证据：本任务clean commit及exact merged production/checker state。production文件、subject registry、checker、
  verify接线或self-test变化会使证据失效。
- integration边界：只提交task branch，不merge main、不push。

## DAG 与执行约束

- 依赖：T07、T08、T09 exact commits已合流integration。
- 解锁：R03。
- branch：`codex/p4-t09r-execution-boundary-registration`。
- worktree：`/Users/geek/workspace/skiff-p4-t09r-checker-registration`。
- 五分钟内真实edit；只修registration/checker integration，不修改Rust/TypeScript production。若仍有production
  violation，回报原owner blocker，不加allowlist。

## 写入范围与完成态

独占execution checker真实subject/required-owner registration、verify/check registry接线和merged-state脚本测试。

1. registration精确覆盖T07 canonical host/request entry与T08 router rejection owner，并能发现文件/registry omission。
2. hermetic self-test保持全PASS；merged production checker violations=0。
3. verify plan中checker恰好展开一次，不与legacy artifact boundary checker重复owner或漏扫Phase 04表面。
4. 不以路径白名单、test-only误判或旧symbol删除掩盖第二dispatcher/TLS/shared-recoverable callback/remote relay。

## 唯一验证 ownership

```bash
node scripts/check-runtime-execution-boundaries.mjs --self-test
node scripts/check-runtime-execution-boundaries.mjs
node --test scripts/tests/runtime-execution-boundary-checker.test.mjs
node scripts/verify.mjs --only checks --list
git diff --check
```

## 回报

提交一个commit，回报exact subject/owner表、merged production零违规、verify展开、命令与自验收矩阵。
