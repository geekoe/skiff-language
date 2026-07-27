# P5-F435A Generic call concrete-return expression typing

状态：Ready。高风险shared compiler checkpoint。

## 直接父节点

- `P5-F434A-aihub-correlation-free-http-combined-result.md`
- `P5-F240-nested-object-literal-target-typing-result.md`

F434A冻结真实AIHub首错、source形状和最小compiler owner；F240冻结target-typed object literal已有
正确边界。引用链继续到唯一权威设计。

## 输入与DAG

| commit | tree |
| --- | --- |
| `9391a895409c88b678c3c50a74a4dc83066540e9` | `318360137a460f2eda4ab00682a58a9438fe2371` |

本节点是AIHub与Agine canonical publish的共享上游。完成后先重跑AIHub combined，再运行Agine
combined；当前候选不是稳定状态。

## 已确认的failure

本地generic helper形如：

```text
function encodeJson<T>(value: T) -> Json
```

caller省略显式type args。`call_type`已找到local callable signature，但
`resolve_callable_signature_type`只有在`type_params.is_empty()`时才解析普通返回文本，导致与`T`
完全无关的明确返回`Json`也被丢弃。随后outer target-typed `JsonObject`只看见call expression
`actual: None`，在AIHub三处报“has no resolved expression type”。

修复不能把任意call加入`expression_accepts_contextual_target`，也不能把field target当成call真实
类型；真实owner是generic callable concrete return fact。

## 写入范围

只允许：

- `compiler/source/src/expression_type_model.rs`
- 必要时在`compiler/source/src/expression_type_model/**`新增或修改direct test module
- 本leaf result

禁止修改AIHub/Agine source、object materialization assignability、其它compiler crate、
runtime/router/test-runner、Internals或skiff-packages。若正确修复需要完整generic inference的新
公共语义或其它production owner，返回`TASK_SCOPE_EXPANDED`。

## 必须实现

1. local generic callable在省略显式type args时，如果声明return type成功解析且最终IR不含任何
   unresolved `TypeParam`，call expression保留该concrete resolved return fact。
2. return直接或嵌套依赖未替换type param时不能伪装成concrete type；`identity<T>(x:T)->T`及
   `Array<T>`等仍须等待合法inference/substitution或保持未解析。
3. 已提供显式type args的exact/structured substitution路径不变。
4. parameter diagnostics、exact package projection、nominal identity、missing/extra/incompatible
   object field检查不被绕过；不得引入`any`或AIHub特例。
5. focused正例覆盖generic helper concrete `Json` return作为：
   - 普通local binding；
   - target-typed object literal field。
6. negative覆盖type-param-dependent return不能被错误具体化；现有F240 unresolved non-identifier
   负例继续失败。
7. 真实AIHub type-check必须越过三处诊断；若出现下一个独立blocker，记录首错与owner，不顺手扩张。

## 验证

本Agent是以下聚焦证据的唯一owner：

```bash
cargo test -p skiff-compiler-source --lib
cargo check -p skiff-compiler-source
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run type-check
cargo fmt --all -- --check
git diff --check
```

AIHub命令使用只读Internals candidate
`/Users/geek/workspace/internals-phase-05-integration`，不得修改它。若type-check越过三处后暴露新
owner，只记录failure classification；完整service test/identity combined仍由F434后继执行。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f435a-generic-return`
- 分支：`codex/p5-f435a-generic-return`

启动后5分钟内完成第一次实际代码修改，或报告具体未知量。提交implementation，再新增并提交
`P5-F435A-generic-call-concrete-return-typing-result.md`。返回commit/tree、focused测试、AIHub
crossing evidence和clean状态。不得merge、rebase、push、stable/live或承接combined。
