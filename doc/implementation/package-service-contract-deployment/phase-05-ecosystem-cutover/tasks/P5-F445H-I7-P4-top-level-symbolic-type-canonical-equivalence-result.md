# P5-F445H-I7-P4 top-level symbolic type canonical equivalence result

状态：

```text
COMPLETE
P4_COMPLETE=YES
DECISION_REQUIRED=NO
SCOPE_EXPANDED=NO
```

## 1. Result

`artifact_symbolic_type_index`不再比较两份合法但写法不同的descriptor。它先以被选择的ABI surface
建立`PackageTypeSymbolIndex`，再将implementation symbol与implementation link都规范化为精确的
package-owned type identity后比较。`topLevel`场景由`implementation_symbols`优先占有source path；
public路径不会反向污染source-only selection。

descriptor内的record、alias、representation、named union与递归type ref全部复用
`normalize_package_interface_type_ref`。`any Interface`的JSON `interface_abi_id`也会先解码、规范化并
重新编码。interface methods独立复用`normalize_package_interface_method_signatures`。

没有新增DTO、artifact/schema代际、fallback或兼容路径。

## 2. RED to GREEN

新增真实package graph fixture覆盖：

- `canonical.CanonicalMessagePointView`的字段引用同package的
  `canonical.CanonicalMessagePoint`；
- `canonical.AgentRuntimeBindings`包含`any canonical.ToolProvider`；
- test package同时以`access: topLevel`依赖subject与agent；
- public path与source path相同。

baseline RED：

```text
package example.com/agent selected type canonical.CanonicalMessagePointView
descriptor disagrees with its implementation link
```

加入`any Interface`后同类RED为：

```text
package example.com/agent selected type canonical.AgentRuntimeBindings
descriptor disagrees with its implementation link
```

修复后fixture通过，并保留public只见API、topLevel只见source的既有负例。

## 3. Fail-closed evidence

就地artifact tests确认以下篡改仍失败：

| 篡改 | 结果 |
| --- | --- |
| interface method签名 | `interface facts disagree` |
| nested package symbol path | `descriptor disagrees` |
| nested package owner | `descriptor disagrees` |
| nested package ABI expectation | `descriptor disagrees` |
| 两个selected type占用同一file/type slot | ambiguous identity |

`isInterface`与`typeParams`仍按原字段精确比较；没有忽略descriptor或放宽缺失link fallback。

## 4. Internals M real evidence

使用当前Skiff worktree、包含M checkpoint且相关Agine/agent source相对
`7fa2ac5de5a576013ee2be74032435a361c8a6e4`无变化的Internals integration tree执行：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f445h-i7-p4-symbolic-type-fix \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node agine/service/test-isolated-service-receipt.mjs
```

真实编译越过原`canonical.CanonicalMessagePointView`以及随后暴露的
`tools.AgentRuntimeBindings` symbolic descriptor首错，最终精确停止在冻结的下游foreign DB blocker：

```text
failed to lower package agine.ai/api-tests source agent_test_support.skiff
db operation target `agent/model.AgentRun` is not a declared db object
in File IR unit expression
```

该预期失败只证明P4已越过；不宣称I7 M或foreign DB闭包完成。

## 5. Verification

```text
cargo test -p skiff-compiler-source type_resolution_model::tests::artifact_
PASS 5/5

cargo test -p skiff-compiler --test package_imports
PASS 12/12

cargo check -p skiff-compiler
PASS

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

额外执行完整`cargo test -p skiff-compiler-source`得到`331/335`。4个失败均位于未修改的既有
reserved-validation/prelude registry或snapshot测试；其中reserved-validation单测独立执行仍在旧fixture
第26行越界。该完整crate结果不作为P4通过证据，也没有在本任务跨scope修复。

以上聚焦gate均由本任务最终提交状态执行。没有访问stable、live、network、Mongo、OAuth或browser，
也没有修改Internals。
