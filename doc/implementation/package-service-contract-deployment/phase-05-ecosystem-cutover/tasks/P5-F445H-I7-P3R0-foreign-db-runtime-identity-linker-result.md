# P5-F445H-I7-P3R0 Foreign DB runtime identity and linker result

状态：

```text
PASS
P3R0_COMPLETE = YES
P3R1_UNBLOCKED = YES
TASK_SCOPE_EXPANDED = NO
DECISION_REQUIRED = NO
```

| 项 | 值 |
| --- | --- |
| implementation commit | `e03db2173e31dc066bf4f7a0aa63cdbad0df164b` |
| implementation tree | `800631ca0d4dda0e495b76d842d271f39f83b00b` |
| baseline | `a2a69a143795987425567e1b51bc92ccf0987c4a` / `4eab9795c3cd13643bd193be586c3e91bc5d0891` |

## 1. Outcome

Runtime linked model新增必填`DbObjectTargetId(PackageArtifactRef, FileIrRef, typeIndex)`。Assembly
conversion在生成linked target时解析该identity；code linker再次按exact artifact/file/type验证并要求
linked type address与identity一致，不能被同名类型或跨artifact替换。

Provider File IR仍是DB metadata唯一owner。本节点没有把collection、key、field、retention、lease、
index或recoverable metadata复制进target；`typeName`只出现在诊断文本。

## 2. Fail-closed matrix

| 场景 | 结果 |
| --- | --- |
| 两个dependency同为`models.User` | PASS；PackageArtifactRef/FileIrRef不同 |
| 非dependency alias target | rejected |
| requirement/binding缺失 | rejected |
| expected build/local ABI mismatch | rejected |
| implementation type export缺失 | rejected |
| provider file/type/DB attachment缺失 | rejected |
| DB attachment未指向同一local type index | rejected |
| artifact/file/type identity替换 | rejected |
| local DB target | 固化为同一`DbObjectTargetId` |
| operation/query/lease claim/lease read carrier | 全部携带同一必填identity |

Transaction自身没有target；其内部DB operation沿用上述carrier。

## 3. RED to GREEN

真实RED：

```text
cargo test -p skiff-runtime-linked-program --locked \
  foreign_db_targets_with_identical_names_keep_exact_dependency_identity
```

测试实际运行1项并失败，错误为`DbTargetResolutionUnavailable`。实现exact resolver后，该项通过。

最终验证：

```text
cargo test -p skiff-runtime-linked-program -p skiff-runtime-linker --locked --no-fail-fast
```

结果：

- linked-program unit：`36/36`；
- linked-program timeout integration：`1/1`；
- linker unit：`61/61`；
- doc tests：`0`项，均成功；
- 总计：`98/98`。

最终代码状态还通过两包locked check、workspace rustfmt check、`git diff --check`及正反向搜索。
既有linker dead-code warning不由本节点引入，不影响结果。

## 4. Scope and handoff

没有修改compiler/source/driver/lowering、Host、Eval、service-db、File IR、PackageArtifact、Local ABI、
ServiceContract generation或外部状态。没有运行stable/live/network/Mongo/OAuth/browser，也没有push。

P3R1可以只消费`DbTargetIr.target_id`并从已admit的provider File IR读取metadata；不得继续按
`typeName`或consumer metadata副本查找。
