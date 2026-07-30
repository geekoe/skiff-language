# P5-F445H-I7-P3R1B Foreign DB Eval declaration alias closure

状态：`IMPLEMENTATION_COMPLETE`。

## 1. Purpose

P3R0B已经接受compiler生成的真实DB声明形状：

```text
declarations.types["Session"].symbol = "model.Session"
declarations.db["Session"].type_ref =
  DbObjectSymbol(modulePath = "model", symbol = "Session")
```

P3R1的Eval resolver仍把声明map key和声明内symbol要求为同一个字符串，因此在DB能力调用前把
上述合法形状误报为`DB target type index declaration is ambiguous`。本节点只闭合Eval
projection：用exact target的file/type index选择唯一声明，验证local/qualified symbol
canonical-equivalent，并继续用local map key读取DB attachment。

## 2. Baseline and scope

| 项 | 值 |
| --- | --- |
| baseline commit | `6def41342e4dd913a9fabd413d1005b0893b09cb` |
| baseline tree | `f3f1b6b47039154d9da47cb28a43dae6a722a919` |
| branch | `codex/p5-f445h-i7-p3r1b-eval-alias` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p3r1b-eval-alias` |
| integration owner | `/root/phase05_integration_steward` |

允许修改：

- `runtime/eval/src/assembly_execution/projection.rs`及其直接测试；
- 违反exact attachment约束的Eval测试夹具；
- 本task/result。

禁止修改：

- compiler、Host、service-db、linked-program/model/schema；
- artifact generation、service boundary、D7 projection；
- stable/live/network/Mongo/OAuth/browser或外部状态。

## 3. Required behavior

- exact `DbObjectTargetId.file_ir_ref + type_index`仍是唯一选择依据；
- 一个type index必须恰好对应一个declaration map entry；
- declaration symbol只允许local map key或`file.module_path + "." + local key`；
- DB attachment必须按local map key读取，不能改成qualified-name lookup；
- linked `Address`、legacy `LocalType`或同文件`DbObjectSymbol`必须指向同一exact type slot；
- 错误qualified symbol、重复slot owner、错误attachment type必须fail closed；
- 不按`typeName`、map迭代first、suffix或全图扫描回退。

## 4. Acceptance

- compiler-qualified declaration direct test先获得真实RED再转GREEN；
- local declaration形状继续通过；
- declaration、duplicate slot和attachment tamper反例通过；
- Eval locked full suite、tests check、rustfmt及baseline diff-check通过；
- P3X compile→assembly→Host→Eval→DB store临时join测试通过。
