# P5-D25：G16 Pre-Host Closure Audit Result

## 熔断与输入

`D25 AUDIT COMPLETE`。G16在同一F04真实路径进入Host前暴露新blocker，完整探针继续暂停。D25A/B/C由三个全新
只读Agent并行/滚动审计harness evidence、Cargo artifact owner与freshness之后的被遮挡范围；没有修改或执行build、
I16、Host、full/stable，也不分别给阶段verdict。权威设计语义未改变。

## 新闭合矩阵

| 跳点 | owner | 输入 → 输出 | 已有正反证据 | 缺口/结论 |
| --- | --- | --- | --- | --- |
| full A build | Cargo + test-only gate | A manifest/shared target → runner/smoke artifacts | G16 build成功；I16三轮origin | 无production blocker证据 |
| B same-selector reuse | Cargo + gate | B manifest/same target → four targeted crates Fresh | G16已到hash/mtime错误，证明四crate Fresh | verbose unit与exact diff未留存 |
| artifact comparator | gate artifact evidence owner | before/outcome/after → stable payload/diff | combined strict comparator PASS | full错误复用包含origin-specific顶层`.d`的全快照 |
| fixture guard | gate evaluator | checked-in source → exact expected assertion | fixture SHA与source guard | 未证明该assertion实际执行 |
| Host attempt | gate state machine | owned command start/outcome → count/result | happy command-double | nonzero/signal/parse failure错误记0 |
| std/Host result | source suite + runner | std/Host运行 → 11/11、1/1、exact PASS line | 分层证据与H18负例 | 本次上游失败遮挡positive；ledger硬编码finalValue |
| inner workspace | isolated runtime owner | temp/config/supervisor → stopped/removed或foreign preserved | lifecycle/PID/outer ownership证据 | foreign替换时仍可能down/status/rm foreign路径 |
| outer cleanup | F18F gate ownership | A/B/task/ledger/process/port → absence | G16本次cleanup全部PASS | 无新blocker；12个旧foreign目录正确保留 |

## 聚合发现

1. `snapshotArtifacts`同时收集稳定binary/rlib、hashed dep-info与root-sensitive顶层`.d`；full模式又用全数组
   hash+mtime相等作为Fresh条件。I16 ledger已证明runner/smoke/compiler三个顶层`.d`随A/B root改变并在final-A恢复；
   Cargo默认dep-info含absolute path。当前最高置信根因是test-only artifact-universe假阴性，不是platform source production
   回归；现场diff丢失，不能把该推断冒充精确路径证明。
2. full失败赋值顺序丢弃before/after diff与`-vv`摘要；Fresh parser只查四个`Fresh`存在，不拒绝同unit
   `Dirty/Compiling`。combined的严格跨根identity快照语义必须保留，不能全局放宽。
3. source-suite一旦启动，非零/signal/parse failure仍会留下`fullProbeRuns:0`；success path只解析两组count，未要求exact
   `PASS main.test.skiff::provider observes helper mutation`，随后硬编码`finalValue`。这些是同一gate evidence owner。
4. inner `/tmp/skiff-test-runtime-*`只有路径，没有nonce/marker/inode owner；teardown前若被foreign替换，会使用其config并
   recursive force删除。outer F18F不能替代inner ownership。local port lease read→remove TOCTOU目前只有残余风险，先在
   本owner便宜测试中尝试复现，无事实前不新建第三修复节点。

## 批量 repair DAG

```text
D25 aggregate checkpoint
├─ F19A gate artifact/evidence convergence
└─ F19B isolated workspace ownership
       ↓ 两个clean commit合流；无在途写入
replacement I16 combined（新schema/candidate，仅一次）
       ↓
fresh narrow acceptance batch + H18 focused-negative
       ↓
fresh R16 → second full-mode gate attempt → new F04 receive
```

F19A与F19B写集互不重叠，可并行。下一次full-mode调用是本周期第二次、原则上的最后一次；第三次前必须重新审计剩余
范围并说明原因。任何节点需要公共契约、架构职责或业务语义变化时暂停受影响分支并请求用户决策；当前无需。
