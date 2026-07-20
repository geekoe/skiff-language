# P5-R01：Canonical Ecosystem Checkpoint Acceptance

## 角色与精确输入

未参与T01实现的只读验收Agent。阅读权威设计 §1–§5、§9–§14，`phase-plan.md`，
P5-T01任务合同，并检查主Agent提供的exact clean integration commit/tree与T01证据。

不得修改文件、创建commit、迁移consumer或重跑已有昂贵gate。

## 必验边界

- authoring DTO分别投影四对象，不共享domain aggregate/kind；contract不带provider/deployment。
- immutable record/path/ref有唯一owner，strict reader/writer不修复未信任identity；path traversal、
  unknown field、missing/duplicate ref及partial write负例可执行。
- active pointer CAS、generation、atomic replace与stale fail-closed精确，没有latest/fallback读。
- Rust/TS control fixture精确一致，只携带environment/generation/assembly/replica；旧per-service
  字段mutation可检出。
- production `RuntimeAssemblyContentResolver` 真实闭合加载四对象，无old graph/index/
  raw JSON semantic inference；T01没有提前迁移host/router/tooling/test-runner。
- checkpoint公开接口足以让T02–T05非重叠并行，未留下会使两个consumer各自
  实现pointer/control schema的空洞。

## 输出

第一行 `PASS` 或 `FAIL`。列blocking issues、non-blocking follow-up、证据命令、动态缺口与残余
风险。PASS只解锁T02–T05，不把该commit称为stable candidate。
