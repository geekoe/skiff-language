# P5-F318 Representation wrap eval consumer

状态：Ready。

## 直接父节点

- linked consumer：`P5-F316-representation-wrap-linked-consumer-result.md`
- local carrier checkpoint：
  `P5-F299-runtime-local-exception-carrier-implementation-result.md`
- handoff audit：
  `P5-F306-representation-constructor-carrier-audit-result.md`

## DAG位置与并行边界

- 节点：representation carrier S3；与F315 compiler producer并行。
- F317同时只改三个open-error fixtures；本任务不得修改：
  - `runtime/eval/src/assembly_execution/websocket_contract_plan.rs`
  - `runtime/eval/src/assembly_execution/ordinary/tests.rs`
  - `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs`
- 完成后与F315进入representation combined probe。

## Production范围

- `runtime/eval/src/eval_context.rs`
- `runtime/eval/src/runtime_ops.rs`
- `runtime/eval/src/type_projection.rs`
- `runtime/eval/src/exceptions.rs`仅exact identity/union branch promotion helper

允许新建不与F317重叠的co-located或integration tests。禁止修改artifact/compiler/linked-program/linker/
linked-type-plan/model/boundary/capability/native/request/host/std。

## 完成标准

### 1. Wrap求值

- `LinkedExprIr::RepresentationWrap`只求值child一次；
- target通过fully-instantiated linked type plan解析为exact Representation；
- 按representation payload plan验证child actual carrier/value；
- 返回保留原raw `RuntimeValue`、仅替换最外层exact representation `CatchIdentity`的carrier；
- plain/generic/external owner与ordered arguments全部进入identity；
- nested explicit wrap逐层验证，outer identity不等于inner identity；
- wrong target/plan、missing identity input、payload identity冲突全部fail closed。

### 2. Exact target-context promotion

- materialize representation carrier到named-union target context时，只在actual exact nominal identity等于
  target branch记录的concrete nominal identity时提升为
  `NamedUnionBranch { union owner+arguments, branch }`；
- 同concrete nominal进入两个不同union时由目标context产生不同branch identity；
- `U<string>`与`U<number>`、同shape不同nominal、literal/synthetic branch均不得误匹配；
- 不按raw shape、display、static throw type或address-only猜；
- ordinary representation materialization不能被union逻辑改变。

### 3. Throw/catch保持

- direct wrap→slot/local return→throw使用actual representation identity；
- exact catch成功，same payload的其它nominal/argument catch miss；
- wrap不创建exception site/frame，initial source/stack仍来自required throw site；
- rethrow保持F299同一Exception语义。

## 验证owner

至少覆盖primitive-backed、generic、nested、external owner、payload conflict、两个enclosing unions与
direct throw/catch。

```bash
cargo test -p skiff-runtime-eval --lib -- --list
cargo test -p skiff-runtime-eval --lib --no-fail-fast
git diff --check
```

若F317未合入造成旧fixture编译遮挡，建立最窄新test target并记录；不得越界修复。不运行root/request/
host/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f318-representation-eval`
- branch：`codex/p5-f318-representation-eval`
- 风险：高；进入A5；
- 一次性Agent，5分钟内修改；提交并返回raw-value/identity/promotion/throw矩阵、反搜与验证；
- 不push、不承接combined probe。

