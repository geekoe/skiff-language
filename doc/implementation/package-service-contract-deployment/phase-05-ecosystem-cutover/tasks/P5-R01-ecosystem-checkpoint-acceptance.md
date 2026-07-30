# P5-R01：Canonical Ecosystem Checkpoint Acceptance

## 角色与精确输入

未参与T01实现的只读验收Agent。阅读权威设计 §1–§5、§9–§14，`phase-plan.md`，
P5-T01任务合同，并检查主Agent提供的exact clean integration commit/tree与T01证据。

不得修改文件、创建commit、迁移consumer或重跑已有昂贵gate。

## 必验边界

- authoring DTO分别投影四对象，不共享domain aggregate/kind；contract不带provider/deployment。
- immutable record/path/ref有唯一owner，strict reader/writer不修复未信任identity；path traversal、
  unknown field、missing/duplicate ref及partial write负例可执行。
- activation state的prepare/commit/abort CAS与crash recovery精确；pre-commit失败保持committed tuple，
  commit后只幂等向前收敛，没有latest/fallback读或持久pointer/runtime active分叉。
- Rust/TS control fixture精确一致，只携带environment/activationId/generation/assembly/replica；旧per-service
  字段mutation可检出。
- production `RuntimeAssemblyContentResolver` 真实闭合加载四对象，无old graph/index/
  raw JSON semantic inference；T01没有提前迁移host/router/tooling/test-runner。
- checkpoint公开接口足以让T02–T05非重叠并行，未留下会使两个consumer各自
  实现activation/control schema的空洞。

## 输出

第一行 `PASS` 或 `FAIL`。列blocking issues、non-blocking follow-up、证据命令、动态缺口与残余
风险。PASS只解锁T02–T05，不把该commit称为stable candidate。

## 执行结果

- `0cebf349`首次FAIL：coordinate codec碰撞、activation shared seam/parity缺失、alias双owner。
- F01后`128af4a7`第二次FAIL：path与alias已关闭；Rust/TS Unicode whitespace值域仍不等价，触发D02
  验收熔断审计。
- D02收齐token/generation/raw JSON/required-nullable/participant/variant同类缺口，F02一次修复；主integration
  combined probe在`c168b1dc`全部PASS。
- 第三次窄复验：`c168b1dc8675eaf42d1062e52dc0cc62a814c2f0` / tree
  `961998aca3022ce5ec3696ff82f1946b04fb7e92`，结论`PASS`，解锁T02–T05。
- `extra-review`无blocking。非阻断观察项：`deployment/src/storage/activation.rs`后续可拆state wire/validation
  与CAS state machine；fixture verifier的decoder dispatch可在不影响证据时去重。
