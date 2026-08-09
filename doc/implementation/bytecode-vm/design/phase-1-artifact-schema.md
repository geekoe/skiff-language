# Phase 1 设计：bytecode artifact schema and structural validator

状态：approved（主 agent 已确认 D1–D19 全部决策，未实现）；依赖 Phase 0 complete

2026-08-10 bracket/index amendment：public contract 已冻结；本文的 initial opcode table 尚不足以
表达嵌套 indexed atomic store/loan，OpcodeContract/schema/runtime implementation pending。

2026-08-10 schema v5 authority amendment：当前 persisted header 必填并精确钉住 opcode contract、native
lifecycle registry、value lifecycle policy、host effect registry 与 intrinsic registry；ISA 保持 v4，
bytecode identity generation 为 v3。此 amendment 只更新 artifact/admission/handoff 契约，不表示 Phase 2–7
实现或阶段验收已完成。

本文是 Phase 1（`phases/phase-1-artifact-schema.md`）的详细设计。它把权威架构契约
`doc/architecture/bytecode-vm.md` 与 requirement ledger 中 Phase 1 部分（R-003、R-009、R-017、
R-018、R-019、R-020、R-022、R-023/Phase 1 部分、R-078/1、R-079、R-080/Phase 1 部分、R-081、
R-220/Phase 1 部分）落成可实现的精确 schema、decoder、validator、identity 与 store path 设计。

本文**不定义新语义**。契约未定义处（operand 具体布局、上限数值、表示法、crate 归属）给出本文的
设计取值，全部集中在 [§9 待主 agent 决策清单](#9-待主-agent-决策清单)。若本设计与架构契约冲突，
以架构契约为准。

---

## 1. 范围与权威输入

### 1.1 本阶段交付面（唯一 schema owner + bounded structural trust boundary）

- 单一 opcode descriptor table：numeric opcode、operand words、允许的 relocation kind、
  operand-stack 签名与 ISA/schema version 由同一 owner 生成或消费（R-017、R-022、R-079）。
- v5 required header pins：opcode contract fingerprint、native lifecycle registry identity、value lifecycle
  policy identity、host effect registry identity 与 intrinsic registry identity 都由各自 canonical owner
  产生，并在 structural admission 前 exact-match；它们不是可选 evidence metadata。
- Relocatable function/template、constant graph、type/shape、frame、exception、resume、
  statement/source、callback capture 与 relocation DTO（R-019、R-020、R-080/1、R-220/1 的
  ValueTransferPlan schema 声明部分）。
- Bounded decoder 与 structural validator：所有 count/offset/index 使用前先校验，整数运算防溢出
  （R-018、R-078/3）。
- Canonical identity/preimage 与 store path 升级（R-003/1）。
- Malformed/corruption corpus 与 property/fuzz 入口（阶段交付物 5，本文 §8 定义组织方式）。

### 1.2 本阶段不实现（边界）

见 [§7 边界与 handoff](#7-边界与-phase-23b-handoff)。关键点：不实现 emitter、linker、
monomorphization、semantic verifier、runtime 执行、decoded micro-op；不保留任何旧 schema reader
作为 fallback（阶段页 §3 非目标）。

### 1.3 权威条款锚点

| 主题 | 架构条款 | 本设计章节 |
| --- | --- | --- |
| wordcode 规则 | §3.2 | §4.1 |
| `RelocatableBytecodeFunction` 字段 | §3.2 | §2.5 / §5.1 |
| relocation 10 类 | §3.4 | §2.4 |
| 42 个指令族 | §3.5 | §2.1–2.3 |
| pre-link 检查清单 | §4.1 | §5.1–5.2 |
| post-link 语义验证（预留） | §4.2 | §7 |
| ValueTransferPlan / R-220 | §6.5 | §2.3 注、§5.1、§7 |
| 两阶段验证边界 | §2.4(3)(5)、§4 | §5、§7 |

> 注：§3.5 的族清单实际包含 42 个不同 mnemonic（Value/slot 6 + Control 5 + Call 6 +
> Callback/interface 4 + Record/value 4 + Collection 10 + Stream 2 + Exception/region 4 +
> Host effect 1）。任务书与部分文档中的“45”与逐项清单不一致，见 [§10 缺口 1](#10-发现的设计契约冲突与缺口)。
> 本设计按 §3.5 权威逐项清单落地 42 个指令，并保留每族 16 个 opcode 槽位作为追加空间
> （R-022 “至少包含所列语义族”）。

---

## 2. Opcode descriptor table（本设计核心）

### 2.1 总则

- 全部指令使用 **wordcode**：函数 code 是 `u32` word sequence；`pc` 是函数 code 内的 word offset；
  每条指令由 1 个 header word（opcode 数值）+ 该 opcode 在该 ISA version 中的**固定** operand word 数构成
  （§3.2）。
- operand 只保存 `immediate`、`relative branch`、`slot/pool/relocation index` 五种 kind，**不保存任何
  runtime address**（§3.2、R-018、R-009）。
- 指令长度、operand kind、stack-effect 声明、允许的 relocation kind 全部来自唯一 `OPCODE_TABLE`
  常量，encoder/decoder/validator 只消费该表（R-022、阶段页 §3 停止条件）。

### 2.2 Numeric opcode 分配方案

按语义族分组，每组固定 16 个槽位（`0xX0`–`0xXF`），族内按 §3.5 出现顺序编号：

| 族 | 范围 | 已分配 | 值 |
| --- | --- | --- | --- |
| Value/slot | `0x00`–`0x0F` | 6 | `const 0x00`、`copy_slot 0x01`、`move_slot 0x02`、`store_slot 0x03`、`drop 0x04`、`dup 0x05` |
| Control | `0x10`–`0x1F` | 5 | `jump 0x10`、`jump_if_true 0x11`、`jump_if_false 0x12`、`switch_tag 0x13`、`budget_checkpoint 0x14` |
| Call | `0x20`–`0x2F` | 6 | `call_local 0x20`、`tail_call_local 0x21`、`call_service 0x22`、`call_actor 0x23`、`call_interface 0x24`、`return 0x25` |
| Callback/interface | `0x30`–`0x3F` | 4 | `interface_box_local 0x30`、`interface_box_remote 0x31`、`make_callback 0x32`、`invoke_callback 0x33` |
| Record/value | `0x40`–`0x4F` | 4 | `new_record 0x40`、`get_dense_field 0x41`、`set_writable_path 0x42`、`representation_wrap 0x43` |
| Collection | `0x50`–`0x5F` | 10 | `new_array_builder 0x50`、`array_builder_push 0x51`、`freeze_array 0x52`、`array_get 0x53`、`array_push_owned 0x54`、`new_map_builder 0x55`、`map_builder_put 0x56`、`freeze_map 0x57`、`map_get 0x58`、`map_put_owned 0x59` |
| Stream | `0x60`–`0x6F` | 2 | `stream_next 0x60`、`emit_stream 0x61` |
| Exception/region | `0x70`–`0x7F` | 4 | `throw 0x70`、`rethrow 0x71`、`enter_region 0x72`、`leave_region 0x73` |
| Host effect | `0x80`–`0x8F` | 1 | `invoke_host 0x80` |
| Reserved | `0x90`–`0xFE` | 0 | 预留（见下） |
| `0xFF` | — | — | 永久 invalid opcode sentinel（不允许分配给任何指令） |

**编号稳定规则（追加永不重编号）**：

1. 新增指令只能在**本族预留槽位**内按序追加；族内 16 槽用尽后，为新增指令分配下一个连续的空闲
   16 槽块作为该族的扩展范围（扩展块仍按族语义分组，便于审查）。
2. 任何已分配 opcode 的数值、operand 布局、stack 签名、允许的 relocation kind 在该 ISA version 内
   不可变；改变它们必须 bump `ISA version`（§2.6）。
3. `0xFF` 保留为 invalid：decoder 遇到 `0xFF` 必须失败（它是文档化的“不可用”值，不是未知 opcode 的
   泛化结果）。
4. header word 的高位 bits 在该 ISA version 内必须为 0（opcode 值域 = `0x00`..=`0xFF`），decoder
   检查；高位立即失败保留给未来更大 opcode 空间，避免历史上“高位=标志位”的兼容陷阱。

### 2.3 每指令 operand 布局与 stack 签名

**Operand kind 词汇**（§3.2 允许的五种）：

| kind | 编码 | 含义 |
| --- | --- | --- |
| `Immediate` | u32 原值 | 立即数（计数类：argCount / captureCount / fieldCount / fieldOrdinal / methodSlot / handlerStackHeight 等；任何语义上“数量”的操作数都走这里） |
| `Branch` | i32，**以 word 计** | relative branch：`targetPc = instructionHeaderPc + 1 + operandWordCount + delta`。delta 以指令 header word 为基准。运算用 checked 语义，targetPc 必须落在 `[0, words.len())` 且指向 instruction header |
| `Slot` | u32 | 函数 frame 内 slot index（含参数 slot），要求 `< frameLayout.slotCount` |
| `Pool` | u32 | artifact 级 pool index（constants / types / shapes / effects / resume / callbackCapture 六类 pool 之一，按 operand 位置固定类别），要求 `< 对应 pool 长度` 且 pool entry kind 与 opcode 要求相容 |
| `Reloc` | u32 | 本函数 `relocations` 数组 index，要求 `< relocations.len()` 且 relocation 的 declared kind ∈ 该 opcode 的 allowed 集合 |
| `Table` | u32 | 本函数辅助表 index（exceptionRegions / switchTables，按 operand 位置固定类别），要求 `< 对应表长度` |

**Stack-effect 表示法**（Phase 1 只是 schema 声明；语义证明归 Phase 3B verifier，§4.2）：

```text
[in1, in2, ...] -> [out1, ...]
```

其中每一项是 `Value(arity)`，arity 取以下来源之一：

- `Fixed(n)`：该 opcode 的固定栈效果数量（如 `dup: [Value(Fixed(1))] -> [Value(Fixed(2))]`）；
- `Declared(operand)`：数量来自某 operand（如 `call_local` 的 `argCount`）；
- `FunctionResultCount`：数量来自本函数 `frameLayout.result_count`（`return` 用）。

列表顺序约定：**自底向上**（`[bottom, ..., top]`）。因此
`call_interface: [Value(Fixed(1)), Value(Declared(argCount))] -> []` 表示 receiver 在栈底、
参数在其上（栈顶是最后求值的参数）。

**42 项指令的完整布局表**（§3.5 逐项清单；operand 顺序即 word 顺序）：

| opcode | 指令 | operand words（布局） | stack in → out | 语义锚点（§3.5 / 其他） |
| --- | --- | --- | --- | --- |
| 0x00 | `const` | `constRef: Pool`（constants，kind=FrozenConstantRef） | `[] -> [Value(Fixed(1))]` | 常量加载；§7 ConstantHeap 由 linker 物化 |
| 0x01 | `copy_slot` | `srcSlot: Slot, dstSlot: Slot` | `[] -> []` | 按 linked `ValueTransferPlan` 执行 share transition（§3.5、§6.5） |
| 0x02 | `move_slot` | `srcSlot: Slot, dstSlot: Slot` | `[] -> []` | 原子转移并清空 source（§3.5、§6.5）；use-after-move 证明归 3B（R-023） |
| 0x03 | `store_slot` | `dstSlot: Slot` | `[Value(Fixed(1))] -> []` | stack 顶部值移入 slot |
| 0x04 | `drop` | `slot: Slot` | `[] -> []` | 执行 linked drop plan（§6.5） |
| 0x05 | `dup` | — | `[Value(Fixed(1))] -> [Value(Fixed(2))]` | share transition，非未追踪 bit copy（§3.5、§1 不变量） |
| 0x10 | `jump` | `target: Branch` | `[] -> []` | |
| 0x11 | `jump_if_true` | `target: Branch` | `[Value(Fixed(1))] -> []` | 弹出条件 |
| 0x12 | `jump_if_false` | `target: Branch` | `[Value(Fixed(1))] -> []` | |
| 0x13 | `switch_tag` | `table: Table`（switchTables）, `defaultTarget: Branch` | `[Value(Fixed(1))] -> []` | 按 nominal/named-union type tag 分派；表项见 §5.1 |
| 0x14 | `budget_checkpoint` | — | `[] -> []` | 语义 charging checkpoint（§16.1/16.2）；自身有固定非零 raw-op 成本 |
| 0x20 | `call_local` | `callee: Reloc`, `argCount: Immediate` | `[Value(Declared(argCount))] -> []` | 同 frame/dispatch loop 继续（§10.2） |
| 0x21 | `tail_call_local` | `callee: Reloc`, `argCount: Immediate` | `[Value(Declared(argCount))] -> []` | 显式 opcode；eligibility 证明归 3B（§3.5、R-081） |
| 0x22 | `call_service` | `serviceOp: Reloc`, `argCount: Immediate`, `resumeRef: Pool`（resumeDescriptors） | `[Value(Declared(argCount))] -> []` | 潜在等待点（§5）；resume 见 §2.3 注 |
| 0x23 | `call_actor` | `actorMethod: Reloc`, `argCount: Immediate`, `resumeRef: Pool` | `[Value(Declared(argCount))] -> []` | |
| 0x24 | `call_interface` | `interfaceReq: Reloc`, `methodSlot: Immediate`, `argCount: Immediate` | `[Value(Fixed(1)), Value(Declared(argCount))] -> []` | receiver 在最底；三 carrier 语义 §12.1；`arity` 属 operand（§12.1） |
| 0x25 | `return` | — | `[Value(FunctionResultCount)] -> []` | 结果数 = `frameLayout.result_count`（ISA v4: 0 或 1） |
| 0x30 | `interface_box_local` | `interfaceReq: Reloc` | `[Value(Fixed(1))] -> [Value(Fixed(1))]` | Local carrier（§12.1） |
| 0x31 | `interface_box_remote` | `serviceOp: Reloc`, `interfaceReq: Reloc` | `[] -> [Value(Fixed(1))]` | RemoteService carrier；serviceOp 提供 dependencySlot + publicInstance 坐标 |
| 0x32 | `make_callback` | `synthetic: Reloc`, `captureCount: Immediate` | `[Value(Declared(captureCount))] -> [Value(Fixed(1))]` | 创建 CallbackClosureRef（§12.2） |
| 0x33 | `invoke_callback` | `interfaceReq: Reloc`, `methodSlot: Immediate`, `argCount: Immediate` | `[Value(Fixed(1)), Value(Declared(argCount))] -> []` | callback carrier（§12.1）；callback 在最底 |
| 0x40 | `new_record` | `shapeRef: Pool`（shapes，kind=ShapeRef）, `fieldCount: Immediate` | `[Value(Declared(fieldCount))] -> [Value(Fixed(1))]` | dense record（§8.4） |
| 0x41 | `get_dense_field` | `shapeRef: Pool`, `fieldOrdinal: Immediate` | `[Value(Fixed(1))] -> [Value(Fixed(1))]` | verified offset 属 3B（§8.4）；预链接只查边界 |
| 0x42 | `set_writable_path` | `rootSlot: Slot`, `shapeRef: Pool`, `fieldOrdinal: Immediate` | `[Value(Fixed(1))] -> []` | 只表达 verified dense-field writable path（§8.3/§6.5）；嵌套 dynamic index 需 OpcodeContract amendment |
| 0x43 | `representation_wrap` | `typeRef: Pool`（types，kind=TypeRef） | `[Value(Fixed(1))] -> [Value(Fixed(1))]` | 包装进 nominal representation / named-union branch |
| 0x50 | `new_array_builder` | `elementTypeRef: Pool`（types） | `[] -> [Value(Fixed(1))]` | transient builder（§8.3） |
| 0x51 | `array_builder_push` | — | `[Value(Fixed(2))] -> [Value(Fixed(1))]` | builder 留在栈上 |
| 0x52 | `freeze_array` | — | `[Value(Fixed(1))] -> [Value(Fixed(1))]` | |
| 0x53 | `array_get` | — | `[Value(Fixed(2))] -> [Value(Fixed(1))]` | strict `Array[integer]`；越界为 source-attributed catchable collection error |
| 0x54 | `array_push_owned` | `slot: Slot` | `[Value(Fixed(1))] -> []` | writable root 在 slot；元素类型来自数组自身类型 |
| 0x55 | `new_map_builder` | `keyTypeRef: Pool`, `valueTypeRef: Pool` | `[] -> [Value(Fixed(1))]` | |
| 0x56 | `map_builder_put` | — | `[Value(Fixed(3))] -> [Value(Fixed(1))]` | |
| 0x57 | `freeze_map` | — | `[Value(Fixed(1))] -> [Value(Fixed(1))]` | |
| 0x58 | `map_get` | — | `[Value(Fixed(2))] -> [Value(Fixed(1))]` | strict `Map[K]`/`JsonObject[string]`；不是 optional `Map.get(key) -> V?` |
| 0x59 | `map_put_owned` | `slot: Slot` | `[Value(Fixed(2))] -> []` | internal upsert；source surface 名为 `Map.set` |
| 0x60 | `stream_next` | `endpointSlot: Slot`, `resumeRef: Pool` | `[] -> [Value(Fixed(1))]` | 一次性 endpoint 下一项（§3.5、§6.5）；affine resource 在 slot |
| 0x61 | `emit_stream` | `resumeRef: Pool` | `[Value(Fixed(1))] -> []` | 真实 backpressure（§3.5、§11.4）；producer 资格证明归 3B |
| 0x70 | `throw` | `typeRef: Pool`（types） | `[Value(Fixed(1))] -> []` | 异常 payload；catch leaf identity 经 type pool（§6.2） |
| 0x71 | `rethrow` | — | `[] -> []` | 需 active exception（3B 证明） |
| 0x72 | `enter_region` | `region: Table`（exceptionRegions） | `[] -> []` | 进入 cleanup region（§13）；pc 归属规则见 §5.1 C6 |
| 0x73 | `leave_region` | `region: Table` | `[] -> []` | |
| 0x80 | `invoke_host` | `effectRef: Reloc`, `argCount: Immediate`, `resumeRef: Pool` | `[Value(Declared(argCount))] -> [Value(Fixed(1))]` | 只有可信 host adapter 可 Pending（§11.1）；adapter identity 语义属 3B |

> **resume 设计注**：§4.2 要求“每个 pending-capable site 有唯一 resume descriptor”；§4.1 的
> jump/switch/handler/**resume** target 检查属于 pre-link。因此当前 schema 把 `resumeRef: Pool` 作为
> pending-capable opcode（`call_service`/`call_actor`/`stream_next`/`emit_stream`/`invoke_host`）的显式
> operand，指向 artifact 级 `resumeDescriptors` pool（`ResumeDescriptor` DTO 见 §5.1）。resume
> descriptor 的语义正确性（唯一性、result/error shape、stack height）仍由 3B verifier 证明；pre-link 只
> 校验 pool 边界与 kind。

#### Bracket/index OpcodeContract amendment

§2.3 的 mnemonic/stack arity 不能单独充当 bracket proof。后续 canonical OpcodeContract 必须同时
编码或从 linked plan 精确解析以下事实：

- receiver 只能是 concrete `Array<T>` / `Map<K,V>` / `JsonObject`；selector 分别是 `integer` /
  exact `K` / `string`，result 分别是 `T` / `V` / `Json`；
- `array_get`/`map_get` 是 strict source bracket，失败生成当前 request 内可 catch 的
  `std.collection.IndexOutOfBoundsError { index, length }` 或不泄露 key 的
  `std.collection.MissingKeyError { container }`；optional `Map.get(key) -> V?` 需独立 semantic
  opcode/intrinsic path，不得共用 strict `map_get`；
- ordinary read 带 exact linked result lifecycle/`ValueTransferPlan`，产生 snapshot，不返回 raw writable
  alias；
- indexed assignment 带有序 segment plan：selector 从外到内各求值一次，然后 RHS 求值一次，
  最后只有一个 atomic store；intermediate 必须 exist，terminal Array 是 replace-only，
  terminal Map/JsonObject 是 upsert；
- `InOut` 按源码 argument 顺序单次求值，全部 path（含 terminal）必须 exist，全部
  selector/path check 后才原子取得整组 loan；callee throw 不回滚已执行 write-through write；
- 每个可失败 segment 都有自己的 `InstructionSourceSite`，`Array.set` 越界使用
  receiver call site；`rethrow` 沿用原 exception
  envelope/source，不在 rethrow pc 重建 source。

失败类型也是 OpcodeContract 的一部分：`Trap(Assertion)` false、divide-by-zero 与非有限
arithmetic 是不可 catch terminal，当前无公开 `ArithmeticError`；不得把它们投影为 collection
error。Runtime-internal `MapEntryAt` 读 canonical snapshot，ordinal 越界是 VM/generated terminal，
不是 source `map[key]` missing。

当前 `set_writable_path` 布局只有 `shapeRef + fieldOrdinal`，`array_push_owned` /
`map_put_owned` 也只指向单 slot receiver；它们无法表达上述多 segment transactional plan 与
per-segment source sites。OpcodeContract owner 必须在 emitter 实现前扩展或增加 canonical
plan/opcode，并按实际 operand/stack semantic 变化 bump ISA/schema。本 amendment 不预分配新 numeric
opcode，也不把未落地工作标为 complete。

**每个指令允许的 relocation kind（§4.1 “relocation declared kind 与使用 opcode 相容”）**：

| opcode | allowed relocation kinds |
| --- | --- |
| `call_local`、`tail_call_local` | `LocalExecutableRef`, `PackageCallableRef`（package-direct call，R-125） |
| `call_service` | `ServiceOperationRef` |
| `call_actor` | `ActorMethodRef` |
| `call_interface`、`interface_box_local`、`invoke_callback` | `InterfaceRequirementRef` |
| `interface_box_remote` | `serviceOp: ServiceOperationRef`, `interfaceReq: InterfaceRequirementRef` |
| `make_callback` | `SyntheticCallbackRef` |
| `invoke_host` | `HostEffectRef` |
| 其余所有指令 | 无 relocation operand（`allowed = []`） |

`TypeRef`/`ShapeRef`/`FrozenConstantRef` 三个 kind 在 initial table 中主要作为 **pool entry** 出现
（`const`→FrozenConstantRef、`new_record`/`get_dense_field`/`set_writable_path`→ShapeRef、
`throw`/`representation_wrap`/`new_array_builder`/`new_map_builder`→TypeRef），也保留为函数级
relocation kind（§3.4 完整清单），未来指令可引用（决策 D11）。

### 2.4 Stack-effect 在 Phase 1 的边界

- stack-effect 是 **schema 声明**：它进入 descriptor table（§2.5）与 artifact 的
  `opcode_table_fingerprint`，供 3B verifier 与 JIT 预留使用（R-009、§3.1）。
- Phase 1 不做 CFG/stack-underflow/merge-point 分析，也不校验声明深度与 `maxOperandDepth`
  的一致性（§4.2 的“重算 max operand depth 不超过 validated declaration”属于 post-link）。
- `ValueTransferPlan`（R-220/Phase 1 部分）在 Phase 1 只落地 **schema 声明**（§5.1 的
  `ValueTransferPlanKind` 四值 + frame/参数/结果/容器位置的 plan 字段）；plan 与 slot kind 的相容
  proof、move-only/affine 的 copy/dup/store 拒绝全部归 3B/6B。

### 2.5 结构化表示（Rust 类型草图，非 public API 承诺）

```rust
// artifact-model/src/bytecode/opcodes.rs —— 单一 owner

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandKind { Immediate, Branch, Slot, Pool, Table, Reloc }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity { Fixed(u16), Declared(&'static str), FunctionResultCount }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackEffect { pub arity: Arity }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationKind {
    LocalExecutableRef, PackageCallableRef, ServiceOperationRef,
    ActorMethodRef, InterfaceRequirementRef, SyntheticCallbackRef,
    HostEffectRef, TypeRef, ShapeRef, FrozenConstantRef,
}

#[derive(Debug, Clone, Copy)]
pub struct OpcodeDescriptor {
    pub opcode: u8,
    pub mnemonic: &'static str,
    pub operand_layout: &'static [OperandKind],     // 顺序 = word 顺序，数量 = operand word 数
    pub stack_in: &'static [StackEffect],
    pub stack_out: &'static [StackEffect],
    pub allowed_relocations: &'static [RelocationKind],
}

pub const OPCODE_TABLE: &[OpcodeDescriptor] = &[ /* 42 项，见 §2.3 表 */ ];

impl OpcodeDescriptor {
    pub const fn operand_word_count(&self) -> u32 { self.operand_layout.len() as u32 }
    pub const fn instruction_word_count(&self) -> u32 { self.operand_word_count() + 1 }
}
pub fn opcode_for(value: u8) -> Option<&'static OpcodeDescriptor>;
pub fn opcode_table_fingerprint() -> String;  // sha256(canonical json of table projection)，见 §2.6
```

- 表是 `const` 静态数组：编译期可见、无初始化顺序问题、无手写第二份长度表。
- encoder/decoder/validator 全部经 `opcode_for`/`instruction_word_count` 取长，禁止手写
  opcode 编号或长度（阶段页 §3 停止条件 2）。
- 表序列化形态：`opcode_table_fingerprint` 用 `skiff_canonical_json` 对投影
  `[{opcode, mnemonic, operandKinds, stackIn, stackOut, relocKinds}]` 求 canonical JSON 再 sha256；
  该指纹随 artifact 携带（§5.1），validator 比对编译期内置指纹（C1）。

### 2.6 ISA/schema version 语义

| 常量 | 当前值 | 用途 |
| --- | --- | --- |
| `BYTECODE_MAGIC` | `"skiff-bytecode"` | 首层 magic 字符串，DTO 头字段 |
| `BYTECODE_SCHEMA_VERSION` | `"skiff-bytecode-v5"` | schema 版本：DTO 形状/字段/表布局与 required authority pins |
| `BYTECODE_ISA_VERSION` | `"skiff-bytecode-isa-v4"` | ISA 版本：opcode 语义 + operand 布局 + stack 签名 |
| `BYTECODE_IDENTITY_SCHEMA_MARKER` | `"skiff-bytecode-artifact-v3"` | canonical identity preimage generation marker |
| `BYTECODE_IDENTITY_PREFIX` | `"skiff-bytecode-image-v3:sha256"` | framed bytecode identity generation |

升级规则：

1. **opcode/operand/stack 语义变化必须升级 ISA version**（§18、R-050）；opcode 数值或 operand 布局
   变化 = ISA 变化。DTO 形状变化（新增表、字段语义变化）升级 schema version；两者通常同 bump。
2. **只追加不重编号**：新增指令用预留槽位（§2.2），在同一 ISA version 内**不允许**，必须 bump；
   bump 后旧 ISA 版本的 artifact 被新 reader 以版本不匹配拒绝。
3. **不保留旧 reader 作为 fallback**（阶段页 §3 非目标）：版本不匹配 = fail closed，不存在
   try-new-then-old、忽略 unknown field/version 或双 reader（阶段页 §5 停止条件 3）。
4. 版本参与 identity preimage（§6.1），因此 schema/ISA 变化必然产生新 bytecode identity 与新
   PackageArtifact build identity；旧 store record 是 immutable 孤儿记录，从不改写（§6.3）。
5. `opcode_table_fingerprint` 与 ISA version 双保险：即使版本字符串被误写，指纹不一致同样拒绝。
6. v5 的五个 required pin 必须作为一个整体保留并 exact-match：
   `opcodeTableFingerprint`、`nativeValueLifecycleRegistry`、`valueLifecyclePolicy`、
   `hostEffectRegistry`、`intrinsicRegistry`。只比较 registry id/version、只验证被当前 CFG 引用的 entry，
   或在 linker/verifier 中改读 ambient authority 都不满足 admission。

2026-08-10 的 v2 schema contract 曾在不改变当时 ISA 的前提下，为 `FrameLayout` 增加必填
`slotTypeRefs`（长度必须等于 `slotCount`）与 `resultTypeRefs`（长度必须等于 `resultCount`）；每项均为
`BytecodePools.types` 中 `TypeRef` entry 的已校验索引。同时，`effectSummaryRef` 的 Rust DTO 类型收紧为
`PackageCallableId`（透明 wire string）。缺字段、长度不一致、越界或非 type entry 一律在 structural
validation 阶段 fail closed。

当前 v5 amendment 在 v4 header 已有的 opcode contract 与 native lifecycle registry pin 之外，新增
value lifecycle policy、host effect registry 与 intrinsic registry 三个 pin。它改变 DTO、structural
admission 和 identity preimage，但不改变 v4 opcode/operand/stack semantics，所以 schema 升到 v5 而 ISA
保持 v4；identity preimage 扩展后 generation 升到 v3。语言尚未发布，不保留旧 schema 或 identity reader。

---

## 3. 模块结构

### 3.1 归属结论

- **不新建 crate**。bytecode schema/decoder/validator 落在 `artifact-model/src/bytecode.rs`（目录
  `artifact-model/src/bytecode/`），与 ledger 倾向（`artifact-model/src/bytecode.rs`，R-003/R-017/
  R-019/R-020/R-022 的 owner）一致，也符合 README §6.1“Prefer a cohesive bytecode schema module”。
- `artifact-model` 已归属 `foundation` subject（`scripts/lib/verify-rust-subjects.mjs`），新增模块
  **不改变唯一归属**，无需改 registry；`--only foundation` 覆盖其测试。
- identity 与 store path 接入 `artifact-identity`（`artifact-identity/src/bytecode.rs` +
  `ecosystem_paths.rs` + `package_artifact.rs`）与 `deployment/src/storage/records.rs`，均仍在各自
  现有 crate/subject 内。

### 3.2 `artifact-model/src/bytecode/` 文件拆分与依赖

```text
artifact-model/src/bytecode.rs            module root：pub use 聚合
artifact-model/src/bytecode/opcodes.rs    只依赖自身
artifact-model/src/bytecode/dto.rs        依赖 opcodes（RelocationKind）+ 既有 types.rs/refs.rs/effects.rs
artifact-model/src/bytecode/encode.rs     依赖 opcodes + dto
artifact-model/src/bytecode/decode.rs     依赖 opcodes + dto（只读 words）
artifact-model/src/bytecode/validate.rs   依赖 opcodes + dto + decode（输出 typed validated view）
artifact-model/src/bytecode/tests/        见 §8
```

单向依赖：`opcodes ← dto ← encode/decode ← validate`。validate 不依赖 encode（corruption corpus
不由 emitter/encoder 生成，阶段页 §4.2）。

各文件主要类型清单：

| 文件 | 主要类型 |
| --- | --- |
| `opcodes.rs` | `OperandKind`、`Arity`、`StackEffect`、`RelocationKind`、`OpcodeDescriptor`、`OPCODE_TABLE`、`opcode_for`、`opcode_table_fingerprint` |
| `dto.rs` | `BytecodeArtifact`、`BytecodeImage`、`BytecodePools`、`RelocatableBytecodeFunction`、`FrameLayout`、`ParameterSlotDecl`、`ValueTransferPlan`/`ValueTransferPlanKind`、`BytecodeRelocation`（10 类 tagged enum）、`BytecodePoolEntry`（6 类 tagged enum）、`ShapeDeclaration`、`FrozenConstantGraph`/`FrozenConstantNode`、`ExceptionRegion`、`CatchMatcher`、`SwitchTable`、`StatementEntry`、`SourceMapEntry`、`ResumeDescriptor`、`CallbackCaptureLayout`、`DebugTable`/`DebugBinding`、`limits` 模块 |
| `encode.rs` | `encode_instruction(opcode, &[u32]) -> Result<Vec<u32>, EncodeError>`（长度取 descriptor）、`assemble_function`、`assemble_artifact`、canonical bytes |
| `decode.rs` | `BoundedDecoder`、`DecodedFunction`、`DecodedInstruction`、`BytecodeDecodeError` |
| `validate.rs` | `StructurallyValidatedView`（opaque token，private fields）、`ValidatedFunction`、`StructuralValidationError`、`structurally_validate(&BytecodeArtifact) -> Result<StructurallyValidatedView, StructuralValidationError>` |

`BytecodeArtifactRef` 放 `refs.rs`（与 `FileIrRef` 同文件，跨 crate 可见）。

### 3.3 `artifact-identity` 接入

```text
artifact-identity/src/bytecode.rs        BytecodeIdentityPayload、bytecode_identity、assign_bytecode_identity、
                                         validate_bytecode_identity（= C1–C8 结构验证 + C9 identity/内容校验）、
                                         ValidatedBytecodeArtifact（opaque token，镜像 ValidatedPackageArtifact 模式）
artifact-identity/src/ecosystem_paths.rs + PackageBytecodeRecordPath
artifact-identity/src/package_artifact.rs build identity projection 增加 bytecode 字段（§6.2）
artifact-identity/src/constants.rs       + BYTECODE_IDENTITY_SCHEMA_MARKER（"skiff-bytecode-artifact-v3"）
                                         + BYTECODE_IDENTITY_PREFIX（"skiff-bytecode-image-v3:sha256"）
```

依赖方向遵守 R-106：`artifact-identity` 只消费 `artifact-model` typed DTO，不反向依赖；
`artifact-model` 不依赖 compiler/runtime/Router。

### 3.4 `deployment` 接入

```text
deployment/src/storage/records.rs   + write_package_bytecode / read_package_bytecode（§6.3）
```

`deployment` 已依赖 `artifact-identity`/`artifact-model`，无需改 Cargo 依赖。

### 3.5 与 Phase 2 emitter / Phase 3B linker 的消费关系

```text
Phase 2 (emitter) ──► artifact-model::bytecode::{opcodes, dto, encode}
                        │
                        ▼
                 artifact-identity::bytecode::assign_bytecode_identity
                        │
                        ▼
            deployment::storage::records::write_package_bytecode
                        │
                        ▼
Phase 3B (linker) ◄── ValidatedBytecodeArtifact / StructurallyValidatedView（唯一可消费形态）
```

Phase 2/3B 能消费的公开 API 清单见 §7.2。本阶段只定义 schema+decoder+validator+identity+store；
emitter/linker 不实现。

---

## 4. Bounded decoder

### 4.1 Wordcode 解码规则

- 输入：`RelocatableBytecodeFunction.words: &[u32]`（已由 DTO 层 bounds 约束，decoder 仍独立
  校验）。`pc` = words 内 word offset（§3.2）。
- 解码是**迭代**的，无递归：
  1. `pc = 0`；`pc < words.len()` 时读 header word `w = words[pc]`；
  2. 校验 `w <= 0xFF`（高位为 0）且 `opcode_for(w as u8)` 存在，否则 `UnknownOpcode`；
  3. `n = descriptor.operand_word_count()`；`checked_add(pc, 1)` 与 `checked_add(pc, 1 + n)`；
     若 `pc + 1 + n > words.len()` → `TruncatedInstruction`（truncated operands）；
  4. 记录 `DecodedInstruction { pc, descriptor, operand_words[0..n] }`，`pc += 1 + n`。
- 输出 `DecodedFunction { instructions: Vec<DecodedInstruction>, header_pcs: Vec<u32> }`
  （header_pcs 升序，供 target 检查二分查找）。
- branch operand 解码：`target = pc + 1 + n + (delta as i32 的 checked 算术)`；越界/非 header 属于
  校验阶段（§5.1 C6），decoder 只负责把它作为签名语义字段原样带出并做溢出安全解码。

### 4.2 校验点（使用前校验）

1. **所有 count/offset/index 使用前先校验**：decoder 内部对 `words` 的全部访问都先做
   `checked_add`/边界比较；任何 pool/relocation/table 访问都发生在 validator 的 bounds check 之后，
   decoder 阶段不存在 artifact-controlled 索引访问（阶段页 §4.2“decoder 在任何
   artifact-controlled 索引访问前失败”）。
2. 指令数上限：`instructions.len() <= MAX_WORDS_PER_FUNCTION`（每指令至少 1 word）。
3. 总资源上限常量（单一 `limits` 模块，全部为受信编译期常量）：

| 常量 | 值 | 约束对象 |
| --- | --- | --- |
| `MAX_ARTIFACT_BYTES` | 256 MiB | canonical JSON 字节数（读取记录时先按字节数拒绝） |
| `MAX_FUNCTIONS` | 100 000 | artifact 函数数 |
| `MAX_WORDS_PER_FUNCTION` | 1 000 000 | 单函数 code words |
| `MAX_RELOCATIONS_PER_FUNCTION` | 100 000 | 单函数 relocations |
| `MAX_TABLE_ENTRIES` | 1 000 000 | 每类辅助表（exceptionRegions/switchTables/statementEntries/sourceMap）条目数 |
| `MAX_POOL_ENTRIES` | 1 000 000 | 每类 pool 条目数 |
| `MAX_SLOTS_PER_FRAME` | 65 536 | frameLayout.slotCount |
| `MAX_OPERAND_DEPTH` | 65 536 | 声明 maxOperandDepth |
| `MAX_ARITY` | 256 | argCount/captureCount/fieldCount/methodSlot 等计数类 Immediate |
| `MAX_NESTING_DEPTH` | 64 | constant graph / type 嵌套深度 |
| `MAX_CONSTANT_GRAPH_NODES` | 1 000 000 | constant graph 节点数 |
| `MAX_CONSTANT_GRAPH_BYTES` | 64 MiB | constant graph 序列化字节 |
| `MAX_SWITCH_TABLE_TARGETS` | 65 536 | 单 switch table target 数 |
| `MAX_TYPE_PARAMETERS` | 64 | 函数 typeParameters 数 |
| `MAX_DEBUG_STRING_BYTES` | 1 MiB | 单个 debug binding/statementId 字符串 |
| `MAX_DEBUG_TABLE_BYTES` | 64 MiB | debug table 总字节 |

4. 嵌套深度：validator 对 constant graph 与 type pool 递归遍历使用显式深度计数（迭代 + 深度上限，
   不用可能爆栈的无界递归；实现用显式 stack）。

### 4.3 错误模型

```rust
pub enum BytecodeDecodeError {
    UnknownOpcode { pc: u32, word: u32 },
    TruncatedInstruction { pc: u32, expected_words: u32, available: u32 },
    ArithmeticOverflow { context: &'static str },          // checked_* 失败
    LimitExceeded { limit: &'static str, actual: u64, max: u64 },
}
```

- 消息形态：结构化 enum + `Display` 输出 `bytecode decode failed at function <key> pc <n>: <kind>`
  风格（与 artifact-model 既有 `FileIrTypeRefValidationError { location, message }` 惯例一致）。
- 失败策略：**任何错误立即中止**整个 artifact 的 decode/validate，不部分成功、不“尽量 link”
  （§4.1）；decode 错误绝无 panic 路径（property 测试保障，§8）。

---

## 5. Structural validator

### 5.1 §4.1 九项检查落成规则（C1–C9）

§4.1 的清单按 R-083 拆成 9 项（“上限”与“溢出”分开计数）：

| # | 检查 | 落成规则 | 数据结构/位置 |
| --- | --- | --- | --- |
| C1 | magic/schema/ISA version 与五个 required authority pin 已知 | `magic == BYTECODE_MAGIC`；`schema_version == BYTECODE_SCHEMA_VERSION`；`isa_version == BYTECODE_ISA_VERSION`；`opcode_table_fingerprint == opcode_table_fingerprint()`；`native_value_lifecycle_registry == native_value_lifecycle_registry_identity()`；`value_lifecycle_policy == value_lifecycle_policy_identity()`；`host_effect_registry == host_effect_registry_identity()`；`intrinsic_registry == intrinsic_registry_identity()`；header word 高位为 0。所有比较都是完整 identity/fingerprint exact equality | `validate.rs::validate_header` |
| C2 | artifact/function/word/table/string/constant graph/nesting/单对象大小在配置上限内 | 逐项对照 §4.2 上限常量表；先检查总体字节数，再按结构逐级检查 | `validate.rs`（`limits` 常量） |
| C3 | 所有 count/offset arithmetic 无溢出 | 全部索引/长度运算走 `checked_add`/`checked_mul`/`checked_sub`；溢出即 `ArithmeticOverflow` | `decode.rs` + `validate.rs` |
| C4 | instruction word 边界完整，opcode operand 数正确 | 每函数执行 bounded decode（§4.1）；任何 `UnknownOpcode`/`TruncatedInstruction` 即失败 | `decode.rs` |
| C5 | local pool/slot/relocation/table index 在界内；relocation declared kind 与使用 opcode 相容 | 逐指令检查每个 operand：`Slot < frameLayout.slotCount`；`Pool < 对应 pool.len()` 且 entry kind 满足该 operand 位置的期望（§2.3 表）；`Reloc < relocations.len()` 且 `relocations[i].kind ∈ descriptor.allowed_relocations`；`Table < 对应表.len()`；计数类 Immediate ≤ `MAX_ARITY` | `validate.rs::validate_operands` |
| C6 | jump/switch/handler/resume target 指向本函数 instruction header | branch operand 目标、switch table targets、`ExceptionRegion.handler_pc`（及 start/end）均 ∈ 本函数 `header_pcs`；`enter_region`/`leave_region` 指令自身 pc ∈ 所引用 region 的 `[start_pc, end_pc)`（决策 D13）；resume descriptor 的 `expected_stack_height` ≤ `MAX_OPERAND_DEPTH` | `validate.rs::validate_targets` |
| C7 | exception/source/statement/capture table 结构有序、无重叠非法区间 | exceptionRegions：`start_pc < end_pc`、按 start_pc 严格升序且 `regions[i].end_pc <= regions[i+1].start_pc`、handler 在函数内；statementEntries：pc 严格升序、去重；sourceMapEntries：`start < end`、按 start 升序、无重叠、在函数 word 范围内；switchTables：targets 全在 header 上、`tag_pool_index` 界内（且 entry kind = TypeRef）；callbackCaptureLayouts：`slot` 与 plan 完整 | `validate.rs::validate_tables` |
| C8 | frozen constant graph bounded、无 cycle、合法 graph encoding | 节点数/字节/嵌套深度 ≤ 上限；**编码约束 `children index < parent index`**（子节点下标严格小于父节点下标，acyclicity 成为纯格式检查，无需搜索算法；决策 D9）；每个 child 下标界内；每个节点引用的 pool/function index 界内且 kind 相容；`FrozenBehavior` 节点引用的 function 存在 | `validate.rs::validate_constant_graph` |
| C9 | artifact identity、内容 hash 与引用记录一致 | identity 部分（§6.2）：声明 `bytecode_identity` == 由 canonical preimage 重算的 identity；record 一致性（store 层）：读到的 canonical bytes 的 sha256 == store 路径 hash；`BytecodeArtifactRef.artifact_path` == canonical record path（`validate_declared_path` 模式） | `artifact-identity/src/bytecode.rs` + `deployment/src/storage/records.rs` |

C1–C8 在 `artifact-model::bytecode::validate`，输出 `StructurallyValidatedView`（opaque token：
字段私有、构造只能经 `structurally_validate` 成功路径，防“caller 自造已校验标记”，镜像
`ValidatedPackageArtifact` 模式）。C9 在 `artifact-identity`，消费 C1–C8 结果并产出最终
`ValidatedBytecodeArtifact`。

C1 必须在“当前 artifact 是否实际引用该 authority”之前执行。否则同一份 wordcode 可以先以一套 lifecycle/
host/intrinsic 语义取得 identity 或通过 handoff，再在另一套 ambient registry 下解释；reachability、link closure
或后续 quickening 的变化还会让原本“未使用”的 entry 变成可执行目标。v5 因此拒绝 partial pin、id/version-only
match 与按需验证，validated view 也必须保留完整 pin，供后续 handoff/hydration/candidate/verifier 逐层交叉核对。

### 5.2 十类 corruption 证明映射（阶段页 §4.2）

| corruption 类 | 覆盖的校验点 |
| --- | --- |
| unknown opcode | C1（高位）、C4（`opcode_for` 查表失败） |
| truncated operands | C4（`TruncatedInstruction`） |
| jump 落入 operand | C6（branch/switch/handler target ∈ header_pcs） |
| index 越界 | C5（slot/pool/reloc/table 全部先界内后访问） |
| 错 relocation kind | C5（`relocations[i].kind ∈ allowed_relocations`；pool entry kind 匹配 operand 位置期望） |
| 重叠 exception/source range | C7（升序 + 无重叠 + handler/区段界内） |
| cyclic/oversized constant graph | C8（child<parent 编码 + 节点/字节/深度上限） |
| count/offset 溢出 | C3（checked 算术） |
| identity/content mismatch | C9（identity 重算比对 + canonical bytes sha256 + record path） |
| 总资源上限 | C2（全部上限常量，含单对象大小） |

### 5.3 validator 输出：typed validated artifact view

```rust
// artifact-model/src/bytecode/validate.rs
pub struct StructurallyValidatedView { /* private fields */ }
impl StructurallyValidatedView {
    pub fn functions(&self) -> &[ValidatedFunction];   // 含 decoded instructions + header_pcs
    pub fn pools(&self) -> &BytecodePools;
    pub fn frozen_constant_graph(&self) -> &FrozenConstantGraph;
    pub fn native_value_lifecycle_registry(&self) -> &NativeValueLifecycleRegistryIdentity;
    pub fn value_lifecycle_policy(&self) -> &ValueLifecyclePolicyIdentity;
    pub fn host_effect_registry(&self) -> &HostEffectRegistryIdentity;
    pub fn intrinsic_registry(&self) -> &IntrinsicRegistryIdentity;
}

// artifact-identity/src/bytecode.rs
pub struct ValidatedBytecodeArtifact { /* private: Arc<BytecodeArtifact>, ref, canonical_bytes, sha256, view */ }
impl ValidatedBytecodeArtifact {
    pub fn admit(artifact: BytecodeArtifact) -> Result<Self>;  // C1–C9
    pub fn artifact(&self) -> &BytecodeArtifact;
    pub fn view(&self) -> &StructurallyValidatedView;
    pub fn reference(&self) -> &BytecodeArtifactRef;
}
```

- **linker（Phase 3B）只能消费 `ValidatedBytecodeArtifact` / `StructurallyValidatedView`**；任何绕过
  structural validation 的路径都不存在（§4.1“失败不得进入尽量 link 路径”、R-083）。
- 失败返回 `StructuralValidationError`（结构化 enum：check 类别 + 定位 + 详情），不做部分结果。

---

## 6. Canonical identity / preimage 与 store path

### 6.1 Bytecode identity preimage 构成

镜像既有 `FileIrIdentityPayload` 模式（`artifact-identity/src/file_ir.rs`）：

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BytecodeIdentityPayload<'a> {
    schema: &'static str,            // "skiff-bytecode-artifact-v3"（identity generation marker）
    schema_version: &'a str,         // "skiff-bytecode-v5"
    isa_version: &'a str,            // "skiff-bytecode-isa-v4"
    opcode_table_fingerprint: &'a str,
    native_value_lifecycle_registry: &'a NativeValueLifecycleRegistryIdentity,
    value_lifecycle_policy: &'a ValueLifecyclePolicyIdentity,
    host_effect_registry: &'a HostEffectRegistryIdentity,
    intrinsic_registry: &'a IntrinsicRegistryIdentity,
    image: &'a BytecodeImage,        // functions + pools + frozen_constant_graph + debug_table
}
```

- 参与 hash 的字节 = `skiff_canonical_json::canonical_json_bytes(payload)`（字段顺序由 struct 定义
  固定；`BTreeMap` 保证 map 顺序规范）。**不包含** `bytecode_identity` 字段自身（自指），其余内容
  全部参与（含 debug table——决策 D14：identity 覆盖全部内容，不引入“可忽略字段”子集）。
- `bytecode_identity = framed_identity(BYTECODE_IDENTITY_PREFIX, sha256_hex(bytes))`，前缀
  `"skiff-bytecode-image-v3:sha256"`。
- schema/ISA version、opcode contract 与四个 registry/policy authority identity 都进入 preimage ⇒ schema
  或任一 semantic authority pin 变化必然改变 bytecode identity（阶段页 §4.2 验收）。这正是 generation
  v3 相对旧 generation 的原子边界；reader 不得用 v3 prefix 重算一个遗漏 authority pin 的旧 preimage。
- 确定性：相同 typed 输入 ⇒ 相同 canonical JSON ⇒ 相同 identity；map/insertion/build 并发顺序由
  `BTreeMap` 消除（阶段页 §4.2 验收“map/insertion/build 并发顺序不改变 canonical identity”）。

### 6.2 与 PackageArtifact build identity / Package Local ABI 的关系

- `PackageArtifact` 新增字段（`artifact-model/src/package_artifact.rs`）：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub bytecode: Option<BytecodeArtifactRef>,
// BytecodeArtifactRef（refs.rs）：{ bytecode_identity: String, artifact_path: Option<String> }
```

- **Package Local ABI identity 不变**：`PackageArtifactLocalAbiIdentityProjection` 只含
  `schema + package_id + public_symbols`，bytecode 字段不进入（R-105 依赖方向、R-125 package-direct
  ABI 事实）；因此 bytecode 内容/格式变化不触碰 Local ABI ⇒ 直接 package 依赖不需要重编译。
- **Build identity 联动**：`PackageArtifactBuildIdentityProjection` 增加
  `#[serde(default, skip_serializing_if = "Option::is_none")] bytecode: Option<BytecodeOwnerIdentityProjection>`
  （`{ bytecode_identity }`）。bytecode 存在时其 identity 进入 build preimage ⇒ 任何 bytecode 内容
  变化 ⇒ 新 PackageBuildId；bytecode 为 None 的既有包（migration 期）build identity 不受影响
  （决策 D18）。
- 接入点：`assign_package_artifact_identities` 在 Local ABI 计算后、build projection 计算前校验
  `bytecode_identity` 已声明且与 `BytecodeArtifact` 内容一致（C9）；`validate_package_artifact_identities`
  同步扩展。

### 6.3 Store path 升级

- 新记录路径（`artifact-identity/src/ecosystem_paths.rs`，镜像 `PackageFileIrRecordPath`）：

```text
records/package-artifacts/<packageId>/<version>/<buildHash>/bytecode/<bytecodeHash>.json
```

  `PackageBytecodeRecordPath::new(package_ref, bytecode_ref)`：`bytecodeHash` 从
  `bytecode_identity` 的 `"skiff-bytecode-image-v3:sha256:"` 前缀后截取；`validate_declared_path`
  校验 `BytecodeArtifactRef.artifact_path` 精确等于 canonical 路径（与 FileIrRef 一致）。

- 存取（`deployment/src/storage/records.rs`）：
  - `write_package_bytecode(&self, package: &PackageArtifactRef, artifact: &BytecodeArtifact)`：
    先跑 C1–C9（identity/内容校验），写 immutable record；**必须先写 bytecode record、后写引用它的
    package record**（读者永远看不到指向缺失 bytecode 的 package record；实现时核对现有 file-ir 的
    写入顺序并保持一致，决策 D19）。
  - `read_package_bytecode(&self, reference: &BytecodeArtifactRef)`：读 bytes → 严格 JSON → typed →
    C1–C8 结构验证 → C9 identity/内容一致 → 返回 `Arc<ValidatedBytecodeArtifact>`；任一失败
    fail closed。
- **旧内容升级规则**：不保留旧 schema/ISA reader 作为 fallback（§2.6 规则 3）。旧记录是 immutable
  孤儿（内容寻址，从不改写）；升级 = 发布新 buildId 的 package record + pointer 原子指向新 buildId
  （既有 `ReleasePointerPath` 机制），旧 schema 的 bytecode 记录自然不再被任何 pointer 引用。Phase 1
  无任何 bytecode reader 进入 production（emitter 未实现），不存在新旧双读路径。

---

## 7. 边界与 Phase 2/3B handoff

### 7.1 本阶段不实现（边界清单）

| 组件 | 归属阶段 | 本阶段只提供 |
| --- | --- | --- |
| Bytecode emitter（控制流/stack effects/relocation 生成） | Phase 2 | schema/encoder API |
| Deployment linker + monomorphization（§3.3） | Phase 3B | relocation DTO + typed validated view |
| Semantic verifier（§4.2 全部 14 项，含 CFG/arity/type/move/`NoPending`/tail-call/resume/budget_checkpoint 覆盖） | Phase 3B | stack-effect schema 声明、`ValueTransferPlan` schema 声明、resume descriptor DTO |
| runtime 执行 / decoded micro-ops（§3.6） | Phase 4+ | — |
| `copy_slot`/`move_slot`/`dup` 的 ownership/share proof（R-023/3B 部分） | Phase 3B | 指令与 plan 声明 |
| ConstantHeap 物化 / ConstEvaluator（§7） | Phase 2/3B | `FrozenConstantGraph` DTO + 校验 |
| ResourceTable / drop plan 语义（R-220/6B 部分） | Phase 6B | `ValueTransferPlan`/drop 字段声明 |
| `emit_stream` producer 资格 / `stream_next` 单消费者 proof | 3B | 指令与 resume operand |
| Bracket/index access、atomic indexed store 与 atomic `InOut` loan plan | OpcodeContract amendment + Phase 2/3B/6B | 现表只有不完整 mnemonic；implementation pending |

### 7.2 Phase 2 只能消费的公开 API 清单

Phase 2（emitter）与后续阶段只能消费以下接口，不能从 runtime decoder 类型反向构造 compiler IR
（阶段页 §6）：

```text
artifact-model::bytecode::opcodes::{OPCODE_TABLE, OpcodeDescriptor, opcode_for,
                                    operand_word_count, opcode_table_fingerprint}
artifact-model::bytecode::dto::{BytecodeArtifact, BytecodeImage, BytecodePools, BytecodeArtifactRef,
                                RelocatableBytecodeFunction, FrameLayout, BytecodeRelocation,
                                BytecodePoolEntry, ShapeDeclaration, FrozenConstantGraph,
                                ExceptionRegion, SwitchTable, StatementEntry, SourceMapEntry,
                                ResumeDescriptor, CallbackCaptureLayout, ValueTransferPlan, limits::*}
artifact-model::bytecode::encode::{encode_instruction, assemble_function, assemble_artifact}
artifact-model::bytecode::validate::structurally_validate
artifact-identity::bytecode::{assign_bytecode_identity, bytecode_identity}
artifact-identity::ecosystem_paths::PackageBytecodeRecordPath
artifact-identity::package_artifact::{assign_package_artifact_identities, ...}   // 已含 bytecode 联动
deployment::storage::records::{write_package_bytecode, read_package_bytecode}
```

Phase 3B 追加消费：`ValidatedBytecodeArtifact`、`StructurallyValidatedView`。

---

## 8. 测试组织（交付物 5：malformed corpus + property/fuzz 入口）

测试文件布局（本阶段只定义组织方式，不实现）：

```text
artifact-model/src/bytecode/tests/
  mod.rs               共享 helper：fixture 构建、canonical bytes、断言工具
  schema_snapshot.rs   golden wire JSON 快照（一个 canonical fixture artifact 的完整 canonical JSON），
                       防止意外 schema/字段变化；schema mutation 断言
  roundtrip.rs         encode → decode → identity 往返确定性；BTreeMap 顺序不敏感；
                       相同输入两次构建得到相同 bytes/identity（阶段页 §4.2 验收）
  corpus.rs            十类 corruption 逐类构造 malformed fixture（手写/变异 canonical fixture，
                       **不由 encoder/emitter 生成**）：unknown opcode、truncated operands、
                       jump 落入 operand、index 越界、错 relocation kind、重叠 exception/source range、
                       cyclic/oversized constant graph、count/offset 溢出、identity/content mismatch、
                       总资源上限；每类至少一个正例 fixture 通过 + N 个负例被拒绝
  limits.rs            每个上限常量做 boundary 测试（at-limit 通过、above-limit 拒绝）
  property.rs          种子化伪随机 word stream 生成器 + 不变式断言：
                       “decode 永不 panic；失败必发生在任何越界访问之前”；
                       固定种子集（CI 内确定性）+ 文档化 fuzz entry fn（供后续 cargo-fuzz 接入，
                       cfg(fuzzing) 导出，不进默认测试）
artifact-identity/src/bytecode/tests.rs
                       identity 确定性/mutation 矩阵（任一字段变化 ⇒ identity 变化）；
                       schema/ISA version 参与 preimage；ValidatedBytecodeArtifact admit/reject；
                       PackageArtifact build identity 联动 + Local ABI 不变性测试
deployment/src/storage/records.rs tests
                       bytecode record 写入/读取、artifact_path canonical 校验、
                       缺失记录/身份不匹配 fail closed
```

测试 gate：新增模块归 `foundation`（无需改 `verify-rust-subjects.mjs`）；`--only foundation`
覆盖；快照/round-trip/corpus/limits/property 与阶段页 §4.1 focused gate 一致。

---

## 9. 决策记录（主 agent 已确认，2026-08-09；D17 后续修订）

D1–D19 的 initial 取值已确认；D17 的版本值随后由 current v5 authority amendment 原子替代，表中记录
当前值。该修订不改变 D17 的原则，也不表示后续执行阶段已验收。

| # | 决策点 | 本文取值 | 影响面 |
| --- | --- | --- | --- |
| D1 | opcode 分组范围 | 9 族 × 16 槽，`0x00`–`0x8F` 已分配 42 个，`0x90`–`0xFE` 预留，`0xFF` invalid | 全 ISA |
| D2 | 变参指令带显式计数 Immediate | `call_local`/`tail_call_local`/`call_service`/`call_actor`/`call_interface`/`invoke_callback`/`new_record`/`invoke_host` 带 `argCount`/`captureCount`/`fieldCount`；`return` 用 `FunctionResultCount` | §2.3 布局 |
| D3 | pre-link 不做跨引用 arity 匹配 | arity 精确匹配（`argCount == callee 参数数` 等）归 3B（§4.2 “arity 精确匹配”）；pre-link 只查 `<= MAX_ARITY` 与 `<= maxOperandDepth` | C5 / 3B |
| D4 | branch 编码 | i32 word delta，基准 = 指令 header；`targetPc = headerPc + 1 + operandWords + delta`；表内 target（switch/handler）用函数内绝对 pc | §2.3 / C6 |
| D5 | pool 与 relocation 双命名空间 | `Pool` 指 artifact 级分类 pool；`Reloc` 指函数级 `relocations`；TypeRef/ShapeRef/FrozenConstantRef 在 initial table 主要作 pool entry kind，同时保留为 relocation kind | §2.3 / §5.1 |
| D6 | resume descriptor 成为显式 operand | pending-capable 五指令带 `resumeRef: Pool`；`ResumeDescriptor { resultTypeRef, expectedStackHeight, resultPlan }` | §2.3 / R-024 预留 |
| D7 | 上限数值 | §4.2 常量表（如 1M words/函数、100k 函数、256 MiB、65536 slots、64 深度） | C2 / limits |
| D8 | 数值表示 | wordcode 嵌入 canonical JSON DTO（`words: [u32]`），artifact 记录仍是 canonical JSON（与 FileIr 同族），不做独立二进制容器 | §6 / store |
| D9 | constant graph acyclic 编码 | `child index < parent index` 格式约束（acyclicity 无需搜索） | C8 |
| D10 | debug table 是否参与 identity | 参与（identity = 除 identity 字段外的全部内容） | §6.1 |
| D11 | 单 artifact 结构 | 每 package 一个 `BytecodeImage`（全部模块函数，function_key 含 module 身份），`PackageArtifact.bytecode: Option<BytecodeArtifactRef>`（migration 期 None） | §6.2 |
| D12 | opcode table fingerprint | sha256(canonical JSON of descriptor 表投影)，artifact 携带、validator 比对 | C1 / §2.6 |
| D13 | `enter_region`/`leave_region` 的 pc 归属规则 | 指令自身 pc 必须在所引用 region 的 `[start_pc, end_pc)` 内 | C6 / §13 |
| D14 | `tail_call_local` 允许的 relocation | 允许 `LocalExecutableRef` 与 `PackageCallableRef`（“exact-local kind”含 package-direct）；eligibility 证明归 3B | §2.3 |
| D15 | `interface_box_remote` 布局 | `[serviceOp: Reloc, interfaceReq: Reloc]`，stack `[] -> [boxed]`（无参数入栈） | §2.3 |
| D16 | `ValueTransferPlan` + typed frame 形态 | `{ kind: SnapshotShare|MoveOnly|AffineResource|ExplicitCloneLease }`，挂在 frame 参数/结果/slot 与 capture/容器类型声明上；v2 frame 另以必填 `slotTypeRefs`/`resultTypeRefs` 对齐 types pool；drop/transfer 细节归 6B | §2.6 / §5.1 / R-220 |
| D17 | identity 前缀与版本字符串（经 v5 authority amendment 更新） | `"skiff-bytecode-image-v3:sha256"` / marker `"skiff-bytecode-artifact-v3"` / schema `"skiff-bytecode-v5"` / ISA `"skiff-bytecode-isa-v4"` / magic `"skiff-bytecode"` | §2.6 / §6.1 |
| D18 | build projection 的 bytecode 字段序列化 | `skip_serializing_if = "Option::is_none"`：无 bytecode 的既有包 build identity 不变 | §6.2 |
| D19 | store 写入顺序 | bytecode record 先于引用它的 package record 写入；实现时核对现有 file-ir 顺序保持同一约定 | §6.3 |

---

## 10. 发现的设计契约冲突与缺口

1. **指令数量不一致**：任务书与多处提及“45 个指令”，但 bytecode-vm.md §3.5 的权威逐项清单只有
   **42 个 mnemonic**（§1.3 计数）。R-022 说“至少包含所列语义族”，42 满足契约；本设计按 42 落地并
   预留 16 槽/族。需主 agent 确认是否有遗漏指令（例如 §3.6 decoded micro-op 中的
   `CallInterface`/`GetDenseField`/`InvokeHost` 是 decoded 形态而非 artifact 指令，不应计入）。
2. **R-019 与最终 §3.2 的 “constants” 字段差异**：R-019 列出函数级 `constants`，最终文本把
   frozen constant graphs 移到 package artifact 级（§3.2 “Package artifact 另保存…”）。以最终文本
   （`430e5ff2`）为准：本设计常量在 artifact 级 pool + `FrozenConstantGraph`，函数不再有 constants
   字段。
3. **R-020 旧词与最终 §3.4 不一致**：R-020 文字含 `InterfaceMethodRef`/`EffectRef`，最终 §3.4 是
   `InterfaceRequirementRef`/`HostEffectRef`（10 类）。以最终文本为准（本设计 §2.4）。
4. **§3.2 函数字段无 resume 表**：架构给 `RelocatableBytecodeFunction` 的字段清单不含 resume
   descriptor，但 §4.1 的 target 检查包含 resume、§4.2 要求每个 pending-capable site 唯一 resume
   descriptor。本设计以 `resumeRef: Pool` + `ResumeDescriptor` DTO 补齐（D6），属于契约未定义处的
   设计取值，需主 agent 确认放 pool 而非函数字段。
5. **`emit_stream`/`stream_next` 的 producer/consumer 资格**是 §3.5/§11.4 语义事实，pre-link 无法
   证明（需要 effect summary/CFG）；本设计把该证明归 3B，schema 只声明指令。验收时需在 3B 明确
   失败证据，不在 Phase 1 声称。
6. **pre-link 与 post-link 的 arity 分工**：§4.2 明确 “arity、parameter/return plan 与 opcode 精确
   匹配” 是 post-link 检查，因此 pre-link 不查 arity 一致性（D3）。这与 “structural validator 是
   linker 前唯一防线” 不冲突：arity 越界无法造成越界访问（C5 已保证索引界内），只可能造成语义
   错误，由 3B 拒绝。
7. **enter/leave_region 的 pc 归属规则在契约中未定义**（D13）：架构只定义 `ExceptionRegion`
   字段与 region 语义。本设计取 “指令 pc ∈ 所引用 region 范围”，需确认。
8. **`interface_box_remote` 是否需要参数入栈**（D15）：契约只说 RemoteService carrier 由
   dependencySlot + publicInstance + operationTable 构成，未定义 box 指令的参数形态；本设计取
   `[] -> [boxed]`。
9. **遗留 `runtime_assembly` 模块**：artifact-model/artifact-identity 仍保留
   `RuntimeAssembly` 相关模块（Phase 8 删除目标）。Phase 1 的 bytecode 模块与它们无依赖关系，
   不触碰；但 `BYTECODE_*` 常量进 `schema.rs` 时注意与既有版本字符串风格统一。
10. **Bracket/index 编码缺口**：2026-08-10 public contract 已冻结，但当前 42-opcode table
    没有嵌套 dynamic path、per-segment policy/source、atomic store 或 atomic multi-loan 的完整表示。
    本文已冻结 OpcodeContract 必须满足的语义，但 schema/ISA 选形与实现仍 pending。
