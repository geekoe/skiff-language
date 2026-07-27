# P5-F445D Timeout syntax checkpoint

状态：Ready。F445B-I1 implementation node。

## 直接父节点

- `P5-F445B-timeout-expression-implementation-preflight-result.md`

## 输入

Skiff integration：

`/Users/geek/workspace/skiff-phase-05-integration` @ `d7596b4b`

production相对F445B预检只可能并入独立interface identity修复；本节点只写 `syntax/**`，不得吸收它。

## 实现合同

在 `syntax/**` 完整建立reference已声明的source surface：

1. duration literal为独立token/AST值，保留原始正整数digits、单位
   `ms|s|m|h|d`和span，并提供checked safe-integer毫秒换算。
2. 接受：
   - `timeout(200ms) { ... }`
   - `value { ... tailExpr }`
   - `concurrent { ... }`
   - `serial { ... }`
   - `concurrent value { ... tailExpr }`
   - `timeout(200ms) value { ... tailExpr }`
   - `timeout(200ms) concurrent value { ... tailExpr }`
3. 只接受canonical modifier顺序；duration参数必须是一个duration token。
4. AST必须显式表达statement/value、timeout wrapper、concurrent value、serial body和tail expression；
   visitor/ast-utils完整遍历所有body/tail，不能把新节点藏成普通call/block。
5. syntax层拒绝稳定的词法/形状错误：
   `0s`、负数、小数、空格拆分、未知单位、safe-ms overflow、缺tail、非canonical modifier、
   timeout缺duration/body。作用域、control-flow、concurrent surface和effect规则留给I2。
6. 不修改 compiler/runtime来临时吞掉新AST。result应列出I2需要补齐的exhaustive consumer，
   但本节点只要求 `skiff-syntax` 自身GREEN。

## Test-first

先新增/修改 parser/lexer/visitor tests，证明当前输入真实RED。至少覆盖F445B的T01、
T02–T04的syntax部分及span/round-trip/visitor。

运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-syntax --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo fmt --check
git diff --check
```

不要运行完整workspace gate、stable/live/network。

## 写集与提交

只允许：

- `syntax/**`
- 本任务result

worktree：

`/Users/geek/workspace/skiff-p5-f445d-timeout-syntax`

branch：

`codex/p5-f445d-timeout-syntax`

先提交implementation，再只新增并提交：

`P5-F445D-timeout-syntax-checkpoint-result.md`

最终clean。不得派子Agent、merge/rebase/push。若完整AST/grammar不能在syntax边界形成稳定表示，
停止并按工作流上报，不得留下只为Agine spelling服务的特殊节点。
