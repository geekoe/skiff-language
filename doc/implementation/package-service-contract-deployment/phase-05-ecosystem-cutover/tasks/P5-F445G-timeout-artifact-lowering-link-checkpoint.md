# P5-F445G Timeout artifact, lowering and link checkpoint

状态：Ready。F445B-I3 implementation node。

## 直接父节点

- `P5-F445B-timeout-expression-implementation-preflight-result.md`
- `P5-F445E-timeout-source-semantics-checkpoint-result.md`
- `P5-F445E-R1-timeout-source-semantics-responsibility-split-result.md`

## 输入

Skiff integration：

`/Users/geek/workspace/skiff-phase-05-integration` @ `b8aba75b`

必须包含F445D AST、F445E source plan、F445F scoped control、F445C identity修复并clean。

## 完成目标

把已验证source plan完整投影到持久 executable 与linked program：

1. 明确IR：
   - statement timeout wrapper：checked `duration_ms`、body、source site；
   - value timeout wrapper：checked `duration_ms`、value expression、source site；
   - user-authored sequential `ValueBlock`；
   - compiled `ConcurrentPlanIr`，含稳定lane source order、kind
     `Statement|Serial|Tail`、dependencies、body/tail引用和source site。
2. Lowering只消费
   `PackageSourceModel::execution_semantics()`；不得重新遍历AST猜lane依赖、kind或duration。
3. 补齐F445E result列出的11个lowering exhaustive site，以及compiled/emission/projection/
   spawn-target/source-site/identity walker的所有新kind。
4. Runtime linked-program/linker：
   - strict解码/转换新wrapper与lane plan；
   - 校验duration非零、source site、lane连续顺序、前向依赖禁止、tail shape/dependency闭包；
   - unknown、legacy或corrupt plan fail closed，runtime不重新推导source semantics。
5. File IR持久格式原子升级：
   - `FILE_IR_SCHEMA_VERSION`：v8 -> v9；
   - `FILE_IR_FORMAT_VERSION`：v6 -> v7；
   - `FILE_IR_OPCODE_TABLE_VERSION`：v1 -> v2；
   - 同步所有canonical identity prefix/hash、fixture/golden、reader/admission检查。
6. Package artifact顶层schema、ServiceContract、runtime assembly schema不因本节点无故变化；
   若实际写入这些DTO，必须有独立版本证据，否则停止上报。
7. 保持timeout本身不新增public callable；`maySuspend`继续由body/call graph事实推导。
8. 文件或模块过长时按稳定职责拆分；不得再生成千行多职责单文件。

## Test-first 与验收

先新增独立 compiler/artifact/linker RED，至少覆盖F445B T02–T04、T17的本层：

- statement/value/concurrent-value精确IR shape；
- source plan到IR逐lane相等；
- sequential value body/tail typing与source site；
- timeout duration checked ms和wrapper composition；
- concurrent statement/serial/tail与dependencies；
- strict serde round-trip/canonical bytes；
- corrupt duration/order/dependency/tail/site/unknown kind拒绝；
- old File IR version fail closed；
- identity稳定与版本变化的精确golden；
- existing non-timeout artifact/public ABI不被无关改变。

至少运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-timeout-ir/build/cargo-target \
  cargo test -p skiff-artifact-model --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-timeout-ir/build/cargo-target \
  cargo test -p skiff-compiler --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-timeout-ir/build/cargo-target \
  cargo test -p skiff-runtime-linked-program --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-timeout-ir/build/cargo-target \
  cargo test -p skiff-runtime-linker --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-timeout-ir/build/cargo-target \
  cargo check -p skiff-compiler
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445g-timeout-ir/build/cargo-target \
  cargo fmt --check
git diff --check
```

若 package名不同，先从workspace manifest解析真实名并在result记录；不得发明或跳过对应owner。
完整source suite的4个已知基线不在本节点顺手修复。

## 写集与提交

只允许F445B-I3范围：

- `artifact-model/**`
- `artifact-identity/**`，仅限 File IR identity prefix、对应 canonical preimage/hash 测试与 golden
- `compiler/core/src/spawn_targets.rs`
- `compiler/compiled/**`
- `compiler/lowering/**`
- `compiler/emission/**`
- `compiler/projection/**`
- `runtime/linked-program/**`
- `runtime/linker/**`
- 本任务直接 compiler/artifact/link tests与result

禁止修改syntax、compiler/source、runtime/request/capability-context/eval/host/native或Internals。

worktree：

`/Users/geek/workspace/skiff-p5-f445g-timeout-ir`

branch：

`codex/p5-f445g-timeout-ir`

先提交implementation，再只新增并提交：

`P5-F445G-timeout-artifact-lowering-link-checkpoint-result.md`

最终clean。不得派子Agent、merge/rebase/push、stable/live/network。若新 executable kind必须扩到
未授权持久DTO，按工作流停止并精确上报，不能静默加字段或兼容。
