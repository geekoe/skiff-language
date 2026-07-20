# P4-F15：Final Gate Mechanical Hygiene

## Blocker、输入与边界

T10@`453c11f`还发现两项非语义gate seam：Phase Rust改动中
`runtime/driver/eval/eval_context/tests.rs`有一处targeted rustfmt差异；仓库baseline的command execution policy漏登记
`scripts/lib/artifact-identity-validation.mjs`既有`spawn` owner，main与candidate均以同一违规失败。另router/type-check
首次执行因独立worktree未materialize声明的pnpm依赖而未启动。

权威输入为P4-T10合同、repo command execution policy及workspace规则。本任务只做格式与既有command owner ledger
登记；依赖安装/复用是T10 gate环境准备，不提交依赖或lockfile变化。不得修改runtime/router production语义、checker
规则或放宽policy。

- 依赖：F12、F13合流；与F14R并行，二者写入域不得重叠。
- 解锁：新T10 stability epoch。
- branch：`codex/p4-f15-final-gate-mechanics`。
- worktree：`/Users/geek/workspace/skiff-p4-f15-gate-mechanics`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 完成态与验证

1. 只对Phase改动Rust文件运行targeted rustfmt；不得格式化或混入Phase外baseline文件。
2. ledger精确登记`artifact-identity-validation.mjs`的`spawn` import、唯一调用、真实owner function/class/reason；policy
   self-test仍能检出missing/duplicate/stale/call-count，不得增加例外。
3. 反向确认validation实现与行为不变，只有ledger/format diff；dependency目录保持ignored且不提交。
4. 本任务只格式化当前integration中已存在的Phase Rust改动；F14R owner必须自行格式化其后续改动，二者合流后由T10
   再做一次targeted/full check。若F14R触及本任务已格式化文件，本任务证据失效并由T10只重验该文件。

```bash
node scripts/check-command-execution-policy.mjs
node --test scripts/tests/command-execution-policy.test.mjs
node --check scripts/lib/command-execution-ledger.mjs
cargo fmt --all -- --check
git diff --check
```

若全仓`cargo fmt --all -- --check`仍命中Phase外baseline，提供main对照并改用Phase文件精确`rustfmt --check`作为
blocking证据；不得顺手格式化无关文件。

## 回报

提交一个clean commit，回报ledger entry、targeted format文件、main对照、命令与自验收矩阵。
