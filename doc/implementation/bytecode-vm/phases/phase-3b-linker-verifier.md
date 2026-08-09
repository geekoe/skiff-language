# Phase 3B: deployment linker, monomorphization and semantic verifier

状态：planned；依赖Phase 3A complete

## 1. 目标

把Phase 2的structurally validated relocatable bytecode链接成concrete、immutable、可安全执行的
`LinkedBytecodeImage`，并由独立post-link verifier证明安全性。本阶段仍不允许production ingress执行未完成VM。

## 2. 交付物

1. Exact Package closure、image-local function/type/shape/const/effect/capability namespace与relocation linker。
2. Canonical specialization worklist；generic template按exact concrete arguments和Self实例化，结果无`TypeParam`。
3. Independent verifier重算CFG、stack height/type、slot liveness、target/arity、effect/`NoPending`、exception/
   resume、tail eligibility、move/share/drop、callback capture与budget checkpoint。
4. Constant graph初始化/冻结计划和atomic image publication；失败时不可观察partial image。
5. Per-build successful/failed load concurrency与cache evidence。
6. v5 authority continuity：hydration与candidate完整保留opcode contract、native lifecycle registry、value lifecycle
   policy、host effect registry与intrinsic registry pins；verifier逐项对照同一个admitted artifact view与当前
   compile-time authority，不接受caller补写或ambient重建。

## 3. 独立性约束

Verifier可以消费canonical opcode schema和linked types，但不得相信emitter/linker声明的stack depth、effect、target、
exception或resource summary。正向artifact由compiler产生；负向corruption corpus必须独立变异raw/validated input。

Header exact pin 与逐-row semantic validation 是两道都必须通过的门禁。C1 先拒绝任何registry/policy fingerprint
mismatch；随后 linker/verifier 对每个 `HostEffectRef` / `IntrinsicRef` 在该精确registry下验证target、binding、
metadata/required context、ABI与instantiated signature，并用精确value lifecycle policy重算相关transfer facts。
只按symbol查找、只比较registry id/version、或“签名看起来兼容”都允许artifact在A authority下取得identity却在B
authority下执行，因此必须fail closed。candidate携带的pin只是待交叉核对的provenance，不是proof。

## 4. 验收

### 4.1 Focused gate

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only checks
git diff --check
```

### 4.2 阶段专属证明

必须覆盖：

- direct/mutual recursion只intern一个specialization；polymorphic expansion和数量/code/type-depth超限稳定失败；
- linked image中`TypeParam`、unresolved target、runtime generic environment数量均为零；
- CFG merge stack/type/liveness不一致、underflow、错arity/return plan、非法tail、`NoPending`可达Pending、错resume
  result、exception overlap、resource copy/use-after-move和无checkpoint cycle全部拒绝；
- 同一validated input在不同map/insertion/concurrency顺序下产生相同linked overlay和image identity；
- 同buildId并发load只发布一个完整image；失败waiter观察同一attempt failure且cache无partial state；
- source/statement tables覆盖call、throw、effect、DB、timeout和generated failure sites。
- opcode/native/policy/host/intrinsic任一header pin mutation均在link/verify前拒绝；host或intrinsic row的target、
  metadata、ABI、instantiated signature与其精确pinned registry不一致也必须独立拒绝。

### 4.3 阶段专属 Live

```bash
node scripts/verify.mjs --only router-live:agine
```

同一轮Agine新artifact必须走decode -> structural validate -> link -> post-link verify并写入manifest；production请求仍
明确走Phase 3A legacy program。Live成功不能被记录为VM执行成功。

## 5. 停止条件

- verifier失败后尝试tree执行、宽松link或runtime lazy specialization。
- linker读取未形成validated view的raw artifact index。
- service provider executable被解析进consumer linked image。
- verifier和emitter共享同一不可独立验证的CFG/stack summary实现。

## 6. Handoff

只有本阶段verifier PASS并原子发布的image可以交给Phase 4 VM。手工构造`Vm::run`输入不属于允许入口。
