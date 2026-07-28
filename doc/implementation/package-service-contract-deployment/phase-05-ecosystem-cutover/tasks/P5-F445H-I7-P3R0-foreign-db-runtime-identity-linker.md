# P5-F445H-I7-P3R0 Foreign DB runtime identity and linker checkpoint

状态：`IMPLEMENTATION_COMPLETE`。

## 1. Purpose

P3D已经冻结test-only foreign DB target的精确链路。本节点只实现runtime linked model和assembly
linker checkpoint：

```text
consumer PackageSymbolRef
  -> PackageRequirement / PackageBinding
  -> exact PackageArtifactRef
  -> implementation_links.types[symbolPath]
  -> provider FileIrRef + typeIndex
  -> provider File IR declarations.db
  -> DbObjectTargetId
```

P3R1后续负责Eval、Host和service-db consumer迁移；本节点不修改这些consumer。

## 2. Baseline and ownership

| 项 | 值 |
| --- | --- |
| baseline commit | `a2a69a143795987425567e1b51bc92ccf0987c4a` |
| baseline tree | `4eab9795c3cd13643bd193be586c3e91bc5d0891` |
| branch | `codex/p5-f445h-i7-p3r0-db-identity` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p3r0-db-identity` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检确认P4活跃写集只有`compiler/source/src/type_resolution_model.rs`与
`compiler/tests/package_imports.rs`，与本节点不重叠。

## 3. Write scope

允许修改：

- `runtime/linked-program`的runtime-only target identity、exact package binding/file/type validation及直接测试；
- `runtime/linker`的File IR conversion、assembly address/link validation及直接测试；
- 本task/result。

禁止修改compiler/source/driver/lowering、Host、Eval、service-db、artifact DTO/schema与任何generation。

## 4. Completion

- `DbObjectTargetId(PackageArtifactRef, FileIrRef, typeIndex)`是每个linked DB target的必填字段；
- operation、`DbQuery`、lease claim/read与transaction内部operation共用该carrier；
- foreign target只接受dependency alias，经exact requirement/binding/build/ABI/link/file/type/DB
  attachment解析；
- local target也在link时固化为同一exact identity；
- `typeName`只用于诊断；
- 缺binding/export/file/type/DB attachment、ABI/build mismatch及cross-artifact substitution全部fail
  closed；
- 两个dependency具有相同module/type名时identity仍不同；
- linked-program/linker locked full suites、check、fmt、diff和反向搜索通过。

