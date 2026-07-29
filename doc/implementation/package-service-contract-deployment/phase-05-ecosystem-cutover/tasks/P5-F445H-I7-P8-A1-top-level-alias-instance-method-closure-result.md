# P5-F445H I7 P8 A1 topLevelAlias instance method closure result

状态：

```text
TASK_SCOPE_EXPANDED
RED_COMPLETE
A1_COMPLETE = NO
AGINE_170_RESUME_UNBLOCKED = NO
DECISION_REQUIRED = NO
```

## 1. Frozen input and RED

零worktree预检与RED锚定Skiff：

```text
baseline commit = 44e83695d5d9e6559b3ac5f482b9faffd1f96cb3
baseline tree   = 6cc2284797d52a6d3549afb255eeaae6247a6915
RED commit      = c05cf4beb56db29e8d441e268fb8b57a044f0852
RED tree        = 67ed93c128353fb1a8f34ea4346b5dd38e6107d8
```

RED同时建立了两层独立失败：

1. projection fixture包含真实`ExecutableKind::ImplMethod`、receiver type和executable declaration；
   `implementationSymbols["api.Worker.handle"]`缺失；
2. compiler driver临时project包含空`api.yml`的provider、`Box`、`makeBox`、`Box.read`和
   `kind: test` consumer；provider artifact已有`internal.Box`与`internal.makeBox`，但
   `implementationSymbols["internal.Box.read"]`缺失，consumer尚不能进入typed source/File IR。

canonical linked fixture也已加入同形`makeBox`/`Box.read`/`box.read()` case，但因projection RED尚未解除而
未冒充execution PASS。

RED命令与非零发现数：

```text
cargo test --locked -p skiff-compiler-projection \
  package_implementation_projection_includes_exact_impl_method_callable -- --nocapture
=> 1 discovered, 0 passed, 1 failed
=> no entry found for key api.Worker.handle

cargo test --locked -p skiff-compiler --test package_imports \
  test_service_top_level_alias_executes_exact_package_receiver_method -- --nocapture
=> 1 discovered, 0 passed, 1 failed
=> no entry found for key internal.Box.read
```

## 2. Scope expansion proof

在冻结owner
`compiler/projection/src/package_artifact/callables/mod.rs`内做最小诊断改动后，projection能够构造：

```text
PackageLocalAbiSymbol::Callable
PackageCallableId(pkg-callable:<package>:top-level:<module>.<type>.<method>)
PackageCallableSignature(self first)
OperationCallableKind::ImplMethod
PackageCallableLinkFact
```

但`project_package_artifact_facts`随即被现有artifact identity validator拒绝：

```text
PackageArtifact is invalid:
implementation link executable Worker.handle targets public callable
pkg-callable:example.pkg:top-level:api.Worker.handle
without a Local ABI signature
```

精确owner是：

```text
artifact-identity/src/package_artifact/validation.rs
  implementation_link_callable_scope
```

该validator遍历`implementation_links.functions + implementation_links.impl_methods`，对同一
File IR executable上的每个`OperationCallableKind::ImplMethod` link只在
`packageLocalAbi.publicSymbols`查signature。权威设计要求同一production source set的impl method也以
独立top-level callable登记到`implementationSymbols`；当impl method同时是现有public-instance method时，
新的top-level link与原public link共享executable，validator必然看到top-level callable id并错误地只查
public surface。

以下绕法均不合法：

- 跳过public-instance对应的impl method：违反“当前impl method namespace”完整投影；
- 把top-level callable伪装为`InternalFunction`：违反冻结的`OperationCallableKind::ImplMethod`；
- 复用public callable id/signature：public-instance signature不含receiver，且Local ABI禁止跨surface重复
  callable id；
- 只让最小private fixture通过：会让含public-instance method的真实package在identity validation失败。

因此完成A1需要新增artifact-identity validator owner。它不要求schema/artifact model代际变化，但已经超出
任务冻结的五个production owner，并触发第7节“其它owner”停止条件。诊断production改动已全部撤销；RED提交
不含production修改。

## 3. Required DAG repair

建议最小新增上游节点：

```text
A1-V artifact identity callable-scope validation
  -> A1 compiler closure
  -> Agine 170 resume
```

`A1-V`只需让implementation link callable scope按精确callable id在public/implementation Local ABI
surface中唯一解析signature，并保留重复id、缺signature、scope不一致fail closed；不得改变schema、identity
代际或public-instance执行语义。完成后A1可在原冻结五个compiler owner继续projection、source typing、
resolved target、receiver-first lowering与五层GREEN。

该扩张是确定性implementation ownership修正，不需要用户设计决策。Runtime、linker、Router、test-runner
production、Agine源码仍保持NO-OP；P8 stream lane仍无因果关系。

## 4. Write set and handoff

本提交实际写集仅为：

```text
compiler/projection/src/package_artifact/tests/fixtures.rs
compiler/projection/src/package_artifact/tests/projection.rs
compiler/tests/package_imports.rs
test-runner/fixtures/alias-return-catch-once/main.skiff
test-runner/fixtures/alias-return-catch-once-tests/main.test.skiff
本task及result
```

分支和worktree保留给integration owner：

```text
branch   = codex/p5-f445h-i7-p8-a1-top-level-receiver
worktree = /Users/geek/workspace/skiff-p5-f445h-i7-p8-a1-top-level-receiver
```

本节点不merge、不push、不运行Agine或J。
