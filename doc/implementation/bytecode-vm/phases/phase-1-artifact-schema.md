# Phase 1: artifact schema and structural validator

状态：candidate-pass（result: [](../results/phase-1.md)）；依赖Phase 0 complete

## 1. 目标

定义bytecode持久格式的唯一schema owner和bounded structural trust boundary。该阶段证明artifact可以被安全、
确定地编码/解码，但不声称任何Runtime已执行bytecode。

## 2. 交付物

1. 单一opcode descriptor table：numeric opcode、operand words、允许的relocation kind、stack-effect descriptor和
   ISA/schema version由同一owner生成或消费。
2. Relocatable function/template、constant graph、type/shape、frame、exception、resume、statement/source、callback
   capture和relocation DTO。
3. Bounded decoder与structural validator：所有count/offset/index使用前先校验，整数运算防溢出。
4. Canonical identity/preimage与store path升级；相同输入确定地产生相同bytes/identity。
5. Malformed/corruption corpus和fuzz/property入口。

## 3. 非目标

- 不执行bytecode，不实现CFG语义验证。
- 不保留旧schema reader作为新reader的fallback。
- 不在compiler/runtime各复制一套opcode编号或instruction length表。

## 4. 验收

### 4.1 Focused gate

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only checks
git diff --check
```

新增artifact/identity crate或模块必须进入唯一verify subject，并通过format/schema snapshot、round-trip、identity
mutation、limit和malformed tests。

### 4.2 阶段专属证明

必须覆盖：unknown opcode、truncated operands、jump落入operand、index越界、错relocation kind、重叠exception/
source range、cyclic/oversized constant graph、count/offset溢出、identity/content mismatch和总资源上限。

验收记录还必须证明：

- decoder在任何artifact-controlled索引访问前失败；
- encoder与validator消费同一schema owner，但corruption tests不由emitter生成；
- map/insertion/build并发顺序不改变canonical identity；
- schema变化必然改变对应artifact/package/deployment build identity，而不无意改变Package Local ABI。

### 4.3 强制 Live

```bash
node scripts/verify.mjs --only router-live:agine
```

本阶段Live是全栈回归，不是bytecode执行证明。Manifest必须明确记录legacy execution和新schema build/test版本，
避免把旧evaluator成功误报为VM成功。

## 5. 停止条件

- validator依赖compiler已验证事实才能避免越界。
- encoder/decoder各自手写opcode长度或编号。
- 需要try-new-then-old reader或忽略unknown field/version才能通过。
- untrusted JSON/bytes在bounded view形成前被整体递归分配或按索引访问。

## 6. Handoff

Phase 2只能消费本阶段公开的schema/encoder API；不能从runtime decoder类型反向构造compiler IR。
