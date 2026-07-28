# P5-F445H-I7-P3R1B Foreign DB Eval declaration alias closure result

状态：

```text
PASS
P3R1B_COMPLETE = YES
P3X_TEMPORARY_JOIN = PASS
TASK_SCOPE_EXPANDED = NO
DECISION_REQUIRED = NO
```

| 项 | 值 |
| --- | --- |
| implementation commit | `70348ed1286d319faea523dcfd1476c2c24bc55c` |
| implementation tree | `d29fd7e309bebab9af640fdcef2dea8ee1bd7f9a` |
| baseline | `6def41342e4dd913a9fabd413d1005b0893b09cb` / `f3f1b6b47039154d9da47cb28a43dae6a722a919` |

## 1. Outcome

Eval DB resolver现在按exact file/type index找到唯一声明map entry，把map key作为local symbol，并允许
声明值使用local symbol或`file.module_path + "." + local symbol`。DB attachment仍按local key
读取，不会把qualified display symbol误用为map lookup key。

resolver同时验证attachment确实拥有同一exact type slot：

- assembly-linked `Address`必须与解析出的`TypeAddr`完全相同；
- legacy `LocalType`必须使用同一type index；
- legacy `DbObjectSymbol`必须使用同一file module和local symbol；
- 其他类型引用全部fail closed。

没有增加`typeName`、suffix、全图扫描或map first fallback。

## 2. RED to GREEN

新增真实compiler形状：

```text
declarations.types["ProjectionType"].symbol = "projection.ProjectionType"
declarations.db["ProjectionType"].type_ref =
  DbObjectSymbol(modulePath = "projection", symbol = "ProjectionType")
```

production修改前运行：

```text
cargo test -p skiff-runtime-eval --locked \
  assembly_db_target_accepts_compiler_qualified_declaration_symbol -- --nocapture
```

实际失败为：

```text
InvalidArtifact("DB target type index declaration is ambiguous")
```

修改后该正例与既有local symbol正例均通过。直接篡改测试还验证以下场景继续被拒绝：

- declaration symbol使用错误module；
- 两个declaration map entry占用同一type index；
- DB attachment指向另一type index。

Eval旧测试夹具的`RawThread` attachment原先错误写成`Json`。完整测试首次运行因此有15项在进入DB
operation前被新的exact校验拒绝；夹具现已改为同文件`svc.main.RawThread`，没有放宽production
规则。

## 3. Verification

Eval完整locked suite：

- unit：`406/406`；
- integration：`4/4 + 5/5 + 6/6`；
- doc：`1/1`。

以下门禁通过：

```text
cargo check -p skiff-runtime-eval --tests --locked
cargo fmt --all -- --check
git diff --check 6def41342e4dd913a9fabd413d1005b0893b09cb
```

## 4. P3X temporary join

P3X test-only commit `63e8a6952043873bd0eec57cba175273972ded0b`临时叠加在R1B
implementation commit之上，运行：

```text
cargo test -p skiff-runtime-host --locked \
  compiled_test_service_foreign_db_targets_reach_exact_host_eval_stores -- --nocapture
```

结果`1/1`通过。compile→assembly→Host admission产生两份同名`Session` metadata，exact
package/file/typeIndex不同；Eval最终分别以exact key调用`first_sessions`和`second_sessions`
store，exists/delete四个事件全部到达。

P3X测试提交未混入R1B正式分支。

## 5. Scope

只修改Eval projection、direct tests、一个违反exact attachment约束的Eval测试夹具及本task/result。
没有修改compiler、Host、service-db、linked-program/model/schema、artifact generation、service
boundary或外部状态；没有运行stable/live/network/Mongo/OAuth/browser，也没有push。
