# P5-F445E Timeout source semantics checkpoint

状态：Ready。F445B-I2 implementation node。

## 直接父节点

- `P5-F445B-timeout-expression-implementation-preflight-result.md`
- `P5-F445D-timeout-syntax-checkpoint-result.md`

## 输入

Skiff integration：

`/Users/geek/workspace/skiff-phase-05-integration` @ `128129ff`

必须包含F445C interface identity修复和F445D syntax，且clean。

## 完成目标

只在 source-semantics边界完整消费F445D AST：

1. `value` body建立词法scope，tail接受外部expected type并决定表达式类型；statement timeout无值，
   value/concurrent-value禁止跨边界 `return|break|continue`，缺失/不兼容tail fail closed。
2. timeout wrapper类型透明，body/tail的call、throw、mutation、root provenance、effect与
   `maySuspend`不被清空或伪造；duration使用F445D checked milliseconds。
3. 完整实现当前 concurrent source surface：
   - 第一层直属statement是lane，直属`serial`整体是一个lane，concurrent-value tail是tail lane；
   - 只允许直属前序 `const` 跨 sibling可见；forward、`let`、nested binding泄漏拒绝；
   - 生成source-order稳定lane DAG与依赖；
   - 拒绝reference列出的control/timeout/value/with/throw/catch/emit/spawn/nested surface；
   - outer mutable-root写入、effect conflict-key、external write冲突和cancel-safety按当前规则fail closed；
   - lane-local fresh root允许。
4. F445D result §5列出的全部production consumer必须显式处理或证明由visitor完整覆盖：
   name/root/target resolution、type/assignability、callable effects、config、stream、package/type、
   DB field path、provider/prelude、contract validation。
5. 在source analysis结果中形成I3可消费的稳定语义计划：
   lane source order、kind、dependencies、tail和source site；不得写artifact或让runtime重新猜source。
6. `compiler/driver/pipeline/mod.rs`注册必要pass；未知新AST路径一律fail closed，不用wildcard跳过。

## Test-first

先新增独立source-semantics测试，当前输入必须真实RED。覆盖F445B T02–T04和I2层面的：

- value expected type/scope/tail/control-flow；
- statement/value timeout checked duration与effect透传；
- sibling const、forward/nested/let；
- serial lane、tail隐式依赖、DAG稳定顺序；
- outer-root mutation与lane-local fresh root；
- read/read可并行、read/write和write/write冲突、exclusive；
-所有非法concurrent surface；
- config/root/call/effect/stream等walker不会漏掉body/tail。

运行至少：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo test -p skiff-compiler-source --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo fmt --check
git diff --check
```

不要求I3前的完整compiler workspace GREEN；result必须列出仍由I3拥有的exhaustive下游，不越界修改。

## 写集与提交

只允许：

- `compiler/source/**`
- `compiler/driver/pipeline/mod.rs`
- 本任务result

worktree：

`/Users/geek/workspace/skiff-p5-f445e-timeout-source`

branch：

`codex/p5-f445e-timeout-source`

先提交implementation，再只新增并提交：

`P5-F445E-timeout-source-semantics-checkpoint-result.md`

最终clean。不得派子Agent、merge/rebase/push、stable/live/network。若完整source semantics必须修改
artifact/lowering/runtime，保留明确I3 handoff并停止本层；不得用临时IR或Agine特例跨边界。
