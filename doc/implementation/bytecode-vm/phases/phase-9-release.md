# Phase 9: release acceptance and performance

状态：planned；依赖Phase 8 complete

## 1. 目标

在唯一VM路径上完成独立release验收、性能门禁和stable rehearsal。Phase 9不承担第一次集成，也不新增语义；
它汇总并挑战Phase 0–8证据。

## 2. 入口条件

- Requirement ledger无`missing`或无owner defer；所有retirement obligation有zero/removal evidence。
- Phase 8已证明所有production ingress只进入新artifact/loader/verifier/VM。
- Phase 0冻结的benchmark workload、机器、profile、统计口径、baseline commit和阈值未在看到结果后修改。
- 三仓候选已合并main并具备可复现的clean-host artifact build输入。

## 3. 验收

### 3.1 完整静态与非Live gate

```bash
pnpm verify
git diff --check
```

若full verify失败，不能用之前各阶段focused PASS拼接成release PASS。修复触及哪个owner，就重开该阶段并重跑
受影响的downstream Live/evidence epoch。

### 3.2 独立语义验收

独立验收者只读检查并抽测：

- source -> artifact -> validated view -> linked image -> VM的单一路径；
- local/remote/callback三carrier、Ready/Pending race、tail/non-tail、throw/unwind、DB-only transaction；
- GC roots、COW/value transfer、constant、resource/drop、memory/fuel limits；
- Actor partial write、exact-build rejection、idle destroy/recreate和durable retry；
- test/production同loader/verifier，legacy/fallback不存在；
- statement/source/profile/error attribution保持reference contract。

验收第一行必须为`PASS`或`FAIL`；开发者自验收、测试数量或前阶段总结不能替代独立判断。

### 3.3 Release benchmark

至少覆盖：

- pure loop、deep local/non-tail/tail calls和Ready unary request；
- dense record、unique Array/Map、nested COW、string/JSON/DB materialization；
- local/remote/callback dispatch、sync child、actual Pending park/resume、pending cleanup；
- stream backpressure、allocation-heavy long request/GC、Actor synchronous segment和suspension；
- 真实Agine chat与strict host-tools profiling。

结果必须包含warmup、样本数、分位数/置信区间、CPU/RSS/allocation/GC/Park指标和baseline/candidate binary SHA。
Correctness-driven regression不得被quickening、sampling窗口或fallback隐藏。未达到Phase 0预注册阈值时Phase 9
失败；阈值修改属于新的评审输入，不能在结果出来后就地放宽。

### 3.4 全量Live与stable rehearsal

```bash
node scripts/verify.mjs --only router-live:http
node scripts/verify.mjs --only router-live:ws
node scripts/verify.mjs --only router-live:actor
node scripts/verify.mjs --only durable-task-e2e-live
node scripts/verify.mjs --only router-live:agine
node scripts/verify.mjs --only router-live:clean-host
```

随后重建/restart main Router/Runtime/Compiler、确认watch发布同一轮fresh artifacts和release pointers，再运行stable：

```bash
cd /Users/geek/workspace/internals/agine
npm run e2e:chat-smoke
npm run e2e:host-tools
```

Host-tools profiling sample、answer和tool call transcript必须非空，runtime PID/SHA与stable candidate一致。

## 4. 完成判定

只有同时满足以下条件才可将项目标为complete：

1. requirement ledger 100%有实现/现存证明/retirement evidence；
2. full verify、独立验收、全部managed Live、stable chat/host和benchmark均PASS；
3. 三仓main commits、artifact/buildIds、binary SHA和全部receipt归档在同一final evidence epoch；
4. 无未解释production legacy命中、fallback、skip、零测试或待补性能门槛；
5. implementation结果文档列出所有剩余非阻塞优化，但没有未关闭correctness/identity/lifecycle风险。
