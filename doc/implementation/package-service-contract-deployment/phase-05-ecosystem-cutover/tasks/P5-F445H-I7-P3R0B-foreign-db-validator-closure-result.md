# P5-F445H-I7-P3R0B Foreign DB validator closure result

状态：

```text
PASS
P3R0B_COMPLETE = YES
P3C_P3R1_TEMPORARY_JOIN_UNBLOCKED = YES
TASK_SCOPE_EXPANDED = NO
DECISION_REQUIRED = NO
```

| 项 | 值 |
| --- | --- |
| implementation commit | `0e66bfaf14a2658eb36e28083dda68a231d2a0e1` |
| implementation tree | `bb627fd16791856037e9d31a45af4af184556a99` |
| baseline | `83bbb5acb73fe338a6005cedb8405e7a7c0cbee2` / `4498ae78ade93372f7a3c53ae7f9fe08eeb1f2b6` |

## 1. Outcome

P3R0原验证器错误地把类型声明map key、声明内symbol、implementation link symbol和DB
attachment都要求成同一个local symbol，并且只接受`LocalType(typeIndex)`。真实compiler File IR
使用以下合法形状：

```text
declarations.types["Session"].symbol = "model.Session"
implementation_links.types["model.Session"].symbol = "Session"
declarations.db["Session"].type_ref =
  DbObjectSymbol(modulePath = "model", symbol = "Session")
```

验证器现在先按唯一`typeIndex`找到canonical declaration，再由File IR module和声明map key得到
`model.Session`。声明内symbol与implementation link symbol允许使用local或qualified表示，但必须
归一到同一个canonical symbol；DB attachment允许同一`LocalType(typeIndex)`或同一File IR
module/local name的`DbObjectSymbol`。

没有放宽package build、File IR identity、type index或artifact边界。错误module、symbol、
type index以及跨artifact替换仍然fail closed。

## 2. RED to GREEN

真实形状正例在production修改前失败：

```text
cargo test -p skiff-runtime-linked-program --locked \
  foreign_db_target_accepts_compiler_qualified_declaration_shape
```

结果为`MissingDbTargetTypeDeclaration`。实现后该正例通过。

新增篡改矩阵覆盖：

- target symbol path错误；
- implementation link symbol错误；
- declaration qualified symbol错误；
- `DbObjectSymbol` module错误；
- `DbObjectSymbol` local symbol错误；
- `LocalType` index错误。

这些反例全部被`DbTargetCanonicalSymbolMismatch`或
`DbTargetAttachmentTypeMismatch`拒绝。既有同名跨dependency、artifact/file/type identity
替换反例继续通过。

## 3. Verification

```text
cargo test -p skiff-runtime-linked-program -p skiff-runtime-linker \
  --locked --no-fail-fast
```

结果：

- linked-program unit：`38/38`；
- linked-program timeout integration：`1/1`；
- linker unit：`61/61`；
- doc tests：`0`项，均成功；
- 总计：`100/100`。

以下门禁也通过：

```text
cargo check -p skiff-runtime-linked-program -p skiff-runtime-linker \
  --tests --locked
cargo fmt --all --check
git diff --check
```

完整测试首次构建因本机磁盘空间不足中止；仅清理本worktree的可重建Cargo产物后，原命令重跑
成功。既有linker dead-code warning未由本节点引入。

## 4. Scope and handoff

本节点只修改runtime linked-program验证器、直接测试和本结果文档。没有修改artifact
generation、DTO、compiler、P3C、P3R1或外部状态；没有运行stable/live/network/MongoDB，也
没有push。

P3C与P3R1可以临时join implementation commit
`0e66bfaf14a2658eb36e28083dda68a231d2a0e1`。正式集成仍由Skiff integration steward执行。
