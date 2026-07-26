# P5-F383 Structured nullable-union alias normalization

状态：Ready。

## 直接父节点

- `P5-F379-openai-nullable-union-audit-result.md`

父节点已冻结现有语言语义并确定唯一owner：透明union alias加`?`必须规范化为外层
`Nullable(Union[...])`。OpenAI source正确；问题是compiler/source把alias RHS格式化为无括号文本后重新
解析。本节点不改变语法或语言设计。

## Worktree与目标

- worktree：`/Users/geek/workspace/skiff-p5-f383-nullable-union-alias`
- branch：`codex/p5-f383-nullable-union-alias`
- base：包含本任务的Skiff phase-05 integration。

实现：

1. `compiler/source/src/type_resolution_model.rs`中的semantic alias expansion保持结构化
   `TypeExpr`/`TypeRefIr`，不得先把union RHS塞进字符串name再format/reparse；
2. source canonicalization对nested union/null/nullable按现有contract规则flatten、sort、dedup并把
   nullable提升到整个base union外；
3. 诊断所需source spelling可以保留，但不得成为semantic identity来源。

不修改OpenAI source、syntax precedence/printer、artifact contract normalization、artifact DTO/identity
schema或语言reference。

## 回归

direct unit至少断言：

- `alias Format = "png" | "jpeg" | "webp"`后`Format?`精确为outer
  `Nullable(Union[3])`；
- `Format?`、`Format | null`及某union分支nullable的等价值集得到同一canonical identity；
- union排序/去重稳定。

end-to-end source至少覆盖：

```skiff
alias Format = "png" | "jpeg" | "webp"
type Request { format: Format? }
function consume(format: Format?) -> void {}
function run(input: Request) -> void { consume(input.format) }
```

正例通过；`consume(format: Format)`接收nullable field必须失败；`"gif"`不得成为成员。并保留已有alias、
artifact descriptor和non-null union回归。

运行：

```bash
cargo test --locked -p skiff-compiler-source nullable_union_alias -- --nocapture
cargo test --locked -p skiff-compiler-source
git diff --check
```

随后使用fresh temporary artifact root bootstrap canonical std并真实publish
`/Users/geek/workspace/skiff-packages-phase-05-integration/openai`。验收：

- OpenAI source零改动；
- publish产生PackageArtifact receipt；
- 不再出现`:465/:471` identity/type mismatch；
- 记录fresh build/Local ABI及结构差异；
- 不访问stable/live/外部OpenAI。

若结构化修复必须改artifact schema或syntax语义，返回`TASK_SCOPE_EXPANDED`。完成production/tests/result
本地commit，worktree clean；不merge/rebase/push。新Agent执行，不派子Agent。
