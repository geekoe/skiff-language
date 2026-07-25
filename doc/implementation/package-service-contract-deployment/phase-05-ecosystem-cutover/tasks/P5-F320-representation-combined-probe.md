# P5-F320 Representation carrier combined probe

状态：Ready。

## 直接父节点

- compiler producer：
  `P5-F315-representation-wrap-compiler-producer-result.md`
- linked consumer：
  `P5-F316-representation-wrap-linked-consumer-result.md`
- eval consumer：
  `P5-F318-representation-wrap-eval-consumer-result.md`
- open-error fixture closure：
  `P5-F317-eval-open-error-contract-fixtures-result.md`

父节点继续引用F306/F308/F299及唯一权威设计。本任务只验证合流后的representation链，并关闭F318已经精确
定位的五处fixture API漂移；不得新增representation或错误通道语义。

## DAG位置与候选

- 节点：representation S4，当前implementation checkpoint上的唯一combined integration probe owner。
- 前置F315/F316/F318已合流；完成后解除representation high-risk acceptance，并为W2-R错误通道提供稳定的
  exact nominal carrier。
- 证据基线是创建worktree时integration HEAD；production representation代码、artifact generation、compiler
  lowering或linked/eval consumer变化都会使本探针失效。

## 写入范围

允许的机械fixture closure：

- `runtime/eval/src/test_effect_registry.rs`：补现有snapshot helper所需的heap参数，不改变snapshot语义；
- `runtime/eval/src/assembly_execution/ordinary/tests.rs`：删除已不存在的`TypeDeclIr.discriminator`和
  `PackageCallableSignature.throw_types`字段，不加替代字段；
- `runtime/eval/src/spawn_ops.rs`：删除已不存在的`LinkedTypeDeclIr.discriminator`字段。

允许新增或补强一个representation full-chain test，范围只能在：

- `runtime/eval/tests/representation_wrap_consumer.rs`；或者
- `runtime/eval/src/assembly_execution/ordinary/tests/`中新建单独模块并做最小mod接线。

禁止修改F315/F316/F318 production代码、artifact/model/generation、compiler、linker、linked-program、
linked-type-plan、request/host/transport/router/std、权威设计。

若合流编译暴露新的同类纯fixture API漂移，可以只记录精确位置并停止；不得扩大写入范围自行修复。

## 完成标准

### 1. 五处fixture closure

- F318记录的五处首错全部清除；
- 只删除/补齐当前required参数，不恢复`throw_types`、`discriminator`或兼容默认；
- `rg -n 'throw_types:|discriminator:'`在本任务三份授权fixture中的结果必须逐项说明，允许与其它现行DTO
  真实字段同名的非旧构造，禁止盲目全删。

### 2. 合流链探针

至少证明：

- source representation constructor经lowering产生required `RepresentationWrap`，payload只出现一次；
- generic/nested/external owner target经File IR和linked IR保留exact owner及ordered arguments；
- eval raw value不变，carrier为exact outer representation；
- direct throw/catch命中exact identity，same payload的其它nominal/argument miss；
- named-union promotion只由目标上下文与exact concrete branch决定；
- required throw site仍是原source/synthetic site，wrap不新增site。

若现有F315/F316/F318测试组合已经覆盖其中某一条，combined owner可以引用并运行对应非零selector；但至少一个
探针必须从编译器产生的真实`ExprIr::RepresentationWrap`继续经过link/eval，而不是三个手造unit模型的简单汇总。

### 3. 负例与generation

- wrong kind/arity/unresolved owner/payload identity conflict失败关闭；
- old File IR generation没有恢复；v8/v6/v8断言保持；
- 无shape/display/static throw fallback、隐式wrap或compat path。

## 唯一验证owner

先关闭编译，再按最小到较宽顺序运行：

```bash
cargo test -p skiff-compiler-lowering --lib -- --list
cargo test -p skiff-compiler-lowering --lib --no-fail-fast
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
cargo test -p skiff-runtime-linker --lib --no-fail-fast
cargo test -p skiff-runtime-linked-type-plan --lib --no-fail-fast
cargo test -p skiff-runtime-eval --test representation_wrap_consumer --no-fail-fast
cargo test -p skiff-runtime-eval --lib -- --list
cargo test -p skiff-runtime-eval --lib --no-fail-fast
git diff --check
```

selector必须非零。某个较早命令失败时收集同一代码状态下独立编译错误，归类后停止，不反复重跑完整eval。
不运行workspace/root/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f320-representation-probe`
- branch：`codex/p5-f320-representation-probe`
- 风险：中等fixture closure + 高风险combined evidence；
- 新的一次性Agent，5分钟内先做三份fixture机械修改，再运行探针；
- 提交fixture/test改动及
  `P5-F320-representation-combined-probe-result.md`，返回commit、精确HEAD、矩阵和剩余blocker；
- 不push、不承接acceptance。
