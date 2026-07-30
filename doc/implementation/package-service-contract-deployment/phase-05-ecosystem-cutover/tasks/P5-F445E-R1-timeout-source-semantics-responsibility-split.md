# P5-F445E-R1 Timeout source semantics responsibility split

状态：Ready。F445E 结构修正；不改变语义。

## 直接父节点

- `P5-F445E-timeout-source-semantics-checkpoint-result.md`

F445E 的语义与测试结果有效，但新增
`compiler/source/src/execution_semantics.rs` 达1951行，并混合：

- public plan model与完整性校验；
- callable external-effect fixed point；
- lexical/concurrent scope、lane DAG与引用验证；
- root provenance、mutation/taint与effect冲突；
- 通用AST collector/diagnostic helper。

这已经构成职责混杂，不能直接作为最终结构合入。

## 修正目标

1. 只做机械/结构性拆分，保持F445E source plan、diagnostic、test和public API行为不变。
2. 使用 `execution_semantics/` 模块边界，至少分离：
   - plan/model/validation；
   - effect profile与fixed point；
   - owner/root/mutation analysis；
   - concurrent lane/scope/reference validation；
   - AST collectors/helpers。
3. 进一步拆分原 `OwnerAnalyzer` 的表达式遍历、concurrent验证和mutation/effect职责；不得把1951行
   原样搬成另一个单一大文件。目标是每个production模块保持可审阅，原则上不超过约600行；
   若某模块仍超过，result必须用单一职责和不可再拆的依赖说明。
4. 不改测试预期、不放宽fail-closed规则、不顺手修4个既有source基线失败或I3 lowering owner。
5. `cargo check -p skiff-compiler`仍应只停在F445E result列出的I3-owned exhaustive sites；
   不得新增source错误。

## 写集与验证

只允许：

- `compiler/source/src/execution_semantics.rs`或其模块目录；
- 仅为module path必要的 `compiler/source/src/lib.rs`；
- 本任务result。

运行同F445E：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo test -p skiff-compiler-source timeout_source_semantics -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo test -p skiff-compiler-source --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo check -p skiff-compiler
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo fmt --check
git diff --check
```

预期仍为：聚焦12/12；完整source 331通过与同4个既有失败；compiler只剩同11个I3 site。

在现有F445E worktree继续，先提交纯重构commit，再只新增并提交：

`P5-F445E-R1-timeout-source-semantics-responsibility-split-result.md`

最终clean。不得派子Agent、merge/rebase/push、stable/live/network。若拆分暴露语义耦合导致不能保持
行为，应停止并精确说明，不得借重构修改合同。
