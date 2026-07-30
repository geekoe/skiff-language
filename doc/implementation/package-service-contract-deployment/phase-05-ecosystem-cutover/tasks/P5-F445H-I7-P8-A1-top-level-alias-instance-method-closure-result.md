# P5-F445H I7 P8 A1 topLevelAlias instance method closure result

状态：

```text
PASS
RED_COMPLETE
A1_V_INTEGRATED
A1_COMPLETE = YES
AGINE_170_RESUME_UNBLOCKED = YES
RUNTIME_LINKER_TEST_RUNNER_PRODUCTION = NO_OP
DECISION_REQUIRED = NO
```

## 1. Frozen ancestry

恢复开发锚定：

```text
resume baseline commit = 6563ef36d7540636fe8e6b28ec4239d8845ec883
resume baseline tree   = 383e9dff8e6b847a79dd899c205ad79f2c7bf293
WIP checkpoint commit  = 067dd442643829ce14bd1bcfa46d77ea2e5771fa
WIP checkpoint tree    = 8f23e24c3bd6d8323061335cc3e7a9447b252e48
```

A1-V原始实现`227cb96f337a0051f279bee9e64a2af0f7068758`已由integration
`405cc99b3af136429a52ecf8b85dbf7d044e5438`合入。为在旧A1工作树上保持精确实现而不对脏改动rebase，
本分支把同一tree cherry-pick为：

```text
local A1-V commit = f8d059fda6dbed7369a5b6c0b19d6807a214d1f6
implementation    = 50b61a85a2e85ed3c6aba6c13c0e70b326684e9c
implementation tree = d4e4cb89866efdcb96713fef845d81ce9cdd073c
```

integration owner应在已经含原始A1-V commit的candidate上串行接入A1 checkpoint与最终implementation，
跳过本地重复的`f8d059fd`。

## 2. Implemented closure

五层链已经闭合：

1. projection把真实`ExecutableKind::ImplMethod`登记为
   `implementationSymbols`、`OperationCallableKind::ImplMethod`、精确
   `PackageCallableId`、callable link和implementation link；signature只保留一个首位`self`。
2. source只为direct `topLevelAlias`产生的精确`PackageSymbol`或完成替换的`AppliedNominal`开放receiver
   method lookup，同时核对Local ABI、build、owner source path与generic arity。
3. resolved target继续使用既有`ResolvedCallTarget::DependencyPackageFunction`，没有增加target种类；
   不按短名或display text回退。
4. lowering继续产生既有`CallTargetIr::PackageCallable`，File IR使用primary alias，执行参数固定为receiver
   第一项，随后才是显式参数；receiver generic arguments排在method generic arguments前。
5. canonical source fixture真实运行`box.read()`并通过7项测试。

top-level view的source type provenance保留在source IR；公开参数和annotation仍优先使用primary public
projection，因此不会破坏既有“top-level callable的公开类型按primary alias兼容”的行为。

## 3. Negative and generic evidence

同一`package_imports`测试族证明：

| 维度 | 结果 |
| --- | --- |
| ordinary public alias | `.read()`不获得implementation method |
| service / permission | 非test service声明`topLevelAlias`被拒绝；service target不进入本分派 |
| method | missing exact member产生结构化source错误 |
| arity / type | 少参、显式重复receiver、错误参数类型均在source阶段拒绝 |
| identity / owner | source unit拒绝错误ABI、错误owner、错误dependency ref及未完成type param |
| generic | `Box<string>`保留exact owner与type argument；0/2 arity和未完成替换均拒绝 |
| existing dispatch | source/lowering现有actor、builtin、local concrete、public instance与interface tests保持原优先级 |

projection使用唯一map key和artifact validator继续对重复/歧义、缺signature及错误link fail closed；A1没有
增加动态method lookup或同名fallback。

## 4. Verification

最终implementation tree上的必需聚焦证据：

```text
cargo test --locked -p skiff-compiler-projection package_artifact -- --nocapture
=> PASS 64/64

cargo test --locked -p skiff-compiler-source package_receiver -- --nocapture
=> PASS 1/1

cargo test --locked -p skiff-compiler-lowering package_receiver -- --nocapture
=> PASS 2/2

cargo test --locked -p skiff-compiler --test package_imports package_receiver -- --nocapture
=> PASS 3/3

cargo check --locked -p skiff-compiler-projection -p skiff-compiler-source \
  -p skiff-compiler-lowering -p skiff-compiler
=> PASS

cargo test --locked -p skiff-compiler --test package_imports
=> PASS 17/17

cargo fmt --all -- --check
=> PASS

git diff --check
=> PASS
```

扩大回归中，`skiff-compiler-source`为`338/342`，4个失败与已记录baseline相同：
reserved-validation越界、两个prelude identity snapshot及builtin spelling owner。
`skiff-compiler-lowering`为`54/55`，唯一失败是既有fixture
`internal/any_lowering.skiff`第35行parser错误；A1聚焦lowering和完整package imports均通过。

canonical命令的目标fixture阶段：

```text
[skiff-tests] running top-level-alias-instance-method
test result: ok. 7 passed; 0 failed
```

随后公共suite在与A1无关的host fixture校验失败：prepare已产出当前
`skiff-runtime-assembly-v3:*`，但`scripts/lib/skiff-source-test-suite.mjs`仍只接受
`skiff-runtime-assembly-v2:*`。该文件在integration `405cc99b`中同样为旧校验，且不在A1冻结写集；
因此不把后续harness漂移冒充A1执行失败，也没有越权修改test-runner production。

Agine完整170项仍归J。A1已解除D2记录的receiver compile缺口；便宜Agine discovery/compile探针由
integration owner在含A1的最终candidate上运行，不能由本旧基线工作树替代。

## 5. Write set and handoff

A1整体production写集严格保持冻结owner：

```text
compiler/projection/src/package_artifact/callables/mod.rs
compiler/source/src/type_resolution_model.rs
compiler/source/src/expression_type_model.rs
compiler/source/src/resolved_call_targets/builder.rs
compiler/lowering/src/function_lowering.rs
```

测试写集为projection fixture/test、`compiler/tests/package_imports.rs`以及两个canonical source fixture。
除此之外只更新本task/result。Runtime、linker、Router、test-runner production、artifact/schema代际、
manifest和Agine业务源码均未修改。

```text
branch   = codex/p5-f445h-i7-p8-a1-top-level-receiver-resume
worktree = /Users/geek/workspace/skiff-p5-f445h-i7-p8-a1-top-level-receiver-resume
```

交给`/root/phase05_integration_steward`串行集成、运行便宜Agine探针并清理本一级worktree/branch；本节点不
merge、不push、不启动J。
