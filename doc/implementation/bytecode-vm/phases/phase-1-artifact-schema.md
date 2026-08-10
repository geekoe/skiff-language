# Phase 1: artifact schema and structural validator

状态：complete（原Phase 1 result: [phase-1](../results/phase-1.md)）；依赖Phase 0 complete。statement-attribution
artifact/identity checkpoint已由`3262d535`/`2c6da16d`落地；它不把Phase 2 compiler、Phase 3B verifier或
Phase 4 VM标为已交付。

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
6. Typed statement authority：`StatementEntry { pc, sequenceOrdinal, attributionId, site }`、same-PC dense
   ordering、fingerprinted default/opcode charge rules、rowless FunctionEntry，以及package-owned full-placement
   manifest identity。

当前schema revision为`skiff-bytecode-v6`：header必填并exact pin opcode contract、native lifecycle registry、
value lifecycle policy、host effect registry与intrinsic registry；typed statement rows替换legacy
`statementId/chargeKind`。default Statement/Expression/Generated charge、rowless FunctionEntry和per-opcode
reclassification均进入opcode fingerprint。opcode/operand/stack semantics未变，所以ISA保持
`skiff-bytecode-isa-v4`；identity为generation v4（`skiff-bytecode-artifact-v4` /
`skiff-bytecode-image-v4:sha256`）。PackageArtifact为`skiff-package-artifact-v14`、Package build identity为
`skiff-package-build-v13:sha256`；required statement
manifest pin进入build preimage但不进入Package Local ABI。这些是artifact/identity contract，不构成Phase 2–7
实现完成证据。

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
- 五个required header pin中任一缺失或变化均在structural admission失败，并改变generation v4 bytecode
  identity；不得只比较registry id/version或忽略当前image未引用的authority。
- same-PC rows的`sequenceOrdinal`从0稠密；typed attribution occurrence无洞；opcode-required class在pc上
  exactly one；legacy`statementId/chargeKind`拒绝。
- statement manifest identity对packageId、全部function origin（含zero-event）及完整placement（含pc）敏感；
  Package-only pin由loader从admitted完整image重算并exact-match。

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
真实函数emission未完成时必须返回structured failure；runtime verifier在immutable statement schedule proof完成前
必须`ProofUnavailable`，VM不得把raw rows当作可执行charge。
