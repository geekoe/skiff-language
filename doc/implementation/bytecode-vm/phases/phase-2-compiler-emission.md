# Phase 2: compiler facts, typed lowering and bytecode emission

状态：planned；依赖Phase 1 complete

## 1. 目标

从source-owned typed facts生成真实relocatable bytecode artifact。建立compiler-owned typed executable MIR/CFG，
不把序列化File IR tree当作新emitter的唯一语义输入。

## 2. 交付物

1. Source model拥有exact expression type、call target/type arguments、callable effect、`maySuspend`、value transfer、
   writable loan/`InOut`、capability provenance、escape和callback capture facts。
2. `var`、immutable `let`、top-level frozen `const`及Package-local `InOut`的parser/static-semantics实现与三仓源码迁移。
3. Typed MIR/CFG拥有slot types、blocks/edges、exception regions、liveness/value-transfer、source/statement entry与
   synthetic callback body。
4. Deterministic emitter生成wordcode、relocations、generic templates、frame/max-stack metadata和constant graph。
   Emitter不接收caller-supplied header override；它从canonical owner精确写入v5的opcode contract、native
   lifecycle registry、value lifecycle policy、host effect registry与intrinsic registry pins。
5. Bounded deterministic const evaluator把top-level const变成frozen constant graph；新artifact不再把const保存为
   request-time executable body。
6. 至少一个真实Package以及完整Agine build closure能输出新artifact并通过Phase 1 structural validator。

## 3. 迁移约束

在VM尚不可执行时，新artifact写入独立临时artifact root或显式validate-only lane；不得把不可执行的新schema发布
到stable release pointer。旧production entry仍可使用显式legacy artifact完成本阶段全栈回归，但不存在
compile/runtime自动fallback。

## 4. 验收

### 4.1 Focused gate

```bash
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only skiff-tests
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only checks
git diff --check
```

三仓所有受新binding语义影响的`.skiff`源码必须完成迁移并在同一compiler candidate上重编译。

### 4.2 阶段专属证明

- 同一source facts重复编译产生相同wordcode/tables/identity；source traversal/map insertion顺序不影响输出。
- emission handoff保留完整v5 authority pins，任一pin mutation都在C1–C9 admission失败；identity使用
  `skiff-bytecode-image-v3:sha256`且ISA仍为`skiff-bytecode-isa-v4`，不得把pin mismatch变成disabled/legacy lane。
- direct/mutual/generic/self calls保留exact symbolic target和canonical type arguments。
- tail position只发射显式`tail_call_local`候选；参数evaluation顺序和source site完整。
- `NoPending`、move-only/affine、use-after-move、非法`InOut`和callback escape负例在source/emission边界拒绝。
- Const cycle、effectful/nondeterministic operation、resource/capability、超出step/depth/size limit均在编译期拒绝；
  相同const输入产生相同frozen graph和identity。
- 真实Agine closure的self-describing manifest从admitted artifact/receipt列出每个bytecode artifact的identity、
  schema、ISA、opcode fingerprint、native lifecycle registry/value lifecycle policy/host effect registry/
  intrinsic registry四个完整authority identity，以及function/word/relocation数量，并全部structural-valid；
  不得只记录matched布尔值或依赖读取时的ambient registry回查。
- 旧evaluator不能作为`var`/value semantics/const/`InOut`变更的正确性oracle；这些使用reference-derived
  golden tests。

### 4.3 强制 Live

```bash
node scripts/verify.mjs --only router-live:agine
```

Live继续验证显式legacy execution lane未被compiler改造破坏；同一轮另在隔离artifact root生成并验证新Agine
bytecode。Manifest必须分开记录两条lane，不能声称chat已在VM执行。

## 5. 停止条件

- emitter必须从File IR恢复缺失type/liveness/effect事实。
- lowering或emitter重新做name/type/conformance inference。
- runtime call-site仍需`TypeParam`推断或临时编译才能解释artifact。
- 为兼容旧源码保留双重`let`/`const`语义。

## 6. Handoff

Phase 3B只消费structurally validated bytecode view；Phase 3A可以并行准备exact-build owner设计，但必须从
Phase 2 complete checkpoint集成。
