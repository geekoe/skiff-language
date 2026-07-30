# P5-D02：Activation Parity Bounded Audit

## 角色、输入与熔断原因

- 只读审计Agent；不是第三次R01 verdict，不修改文件、不创建commit、不迁移consumer。
- 精确输入：integration commit `128af4a7d638026b90827c73f902c2dd66f3e79a` / tree
  `85773fa60ba02711e9313a2a5c7dbbbe97dd229b`，以及P5-R01两次FAIL报告。
- R01第二次FAIL触发`multi-agent-development.md`验收熔断。已关闭的coordinate codec与alias owner不重审；
  本审计只穷举剩余activation跨语言parity范围，为一次repair wave和第三次verdict提供边界。

## 有界审计面

逐项列出Rust与TypeScript的真实production/public decoder入口、共同规则owner、未覆盖mutation与最小复现：

1. `AssemblyActivationRequest`、`AssemblyActivationControl`、`EnvironmentActivationState`的所有Rust
   deserialize/validate入口，以及TS request/control/state decoder；识别调用者可绕过的裸derive、只验证fixture
   不验证production入口、或相同规则的重复实现。
2. token值域：空串、ASCII/Unicode首尾/内部空白、BOM/NBSP/zero-width、C0/C1/Unicode control、UTF-8
   byte长度与JS UTF-16长度、Unicode normalization、lone surrogate/non-scalar输入。
3. generation值域：负零、fraction/exponent、`Number.MAX_SAFE_INTEGER`边界、u64边界、JSON parse rounding与
   expected+1 overflow。
4. assembly identity、schema version、reject reason、exact fields、missing/unknown/duplicate raw JSON key的
   两端等价性；明确哪些属于typed-object decoder能力，哪些必须由raw JSON trust boundary owner处理。
5. participant集合：空/重复/排序，以及Rust byte/code-point ordering与JavaScript UTF-16 ordering差异。
6. cross-system fixture是否让同一mutation corpus真实进入两端decoder；列出当前51项遗漏与最便宜的
   post-merge combined probe，不重跑已通过完整crate/gate。

## 输出

回报：

- `入口 | Rust规则 | TS规则 | parity结论 | mutation证据`矩阵；
- 所有remaining blocking findings，而不是遇到第一个就停止；
- 一次F02 repair的最小写入owner、明确值域建议及受影响证据；
- 第三次R01只需复验的exact checklist。

若发现需要改变四对象、activation transaction或DAG的设计缺口，单列为需要用户决定；否则明确说明只是
既定设计下的shared codec实现修复。
