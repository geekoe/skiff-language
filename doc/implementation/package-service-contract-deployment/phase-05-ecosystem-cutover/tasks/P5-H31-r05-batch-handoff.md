# P5-H31：R05 Batch Handoff

状态：Implementation Checkpoint + I30 PASS，停在最终R05批次之前。未push、未merge main、未操作stable。

## 精确代码状态

- integration worktree：`/Users/geek/workspace/skiff-phase-05-integration`
- branch：`codex/package-service-phase-05`
- production commit：`4a7b145396dc1359d0581d06e0bda1c31718504f`
- production tree：`e0202d962d2580a89871bf5066972d3787b70714`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`
- 唯一允许的untracked：`.p5-i16-combined-ledger.json`，历史SHA-256
  `1cf4dbd25ab5c7ea4701b84245f077b3739691e746aedc17d22a8b03e9d3f364`

本handoff及result文档提交只改变文档；新对话必须重新记录最终HEAD/tree，并确认上述production commit到handoff HEAD无
production diff。

## 本批次完成

- R30真实单generation WS smoke PASS，随后R24 owner checkpoint经F29A窄修复PASS；
- F23E TS/Rust generation lifecycle shared wire完成；
- F03B Router store/snapshot/gateway/participant/pin/release/drain consumer完成；
- F03C Runtime acquire/release/session cleanup/old-generation pin/retire consumer完成；
- D40/F30A关闭explicit compiler sidecar path及local/isolated/remote provisioning；
- I30在production commit上92/92 PASS，Router type-check、Runtime check/DAG、diff/status均PASS。

## 证据有效性

仍有效：

- R25 canonical WS shape与R27 target-object materialization；
- F23E两侧wire/corpus direct evidence；
- F03B/F03C/F30A各自direct evidence；
- I30 exact combined evidence。

仅作历史证据、不能代替最终R05：

- R30发生在F03B/F03C/F30A之前，证明当时的single-generation真实marker，但被后续Router/Runtime/provisioning production
  变化失效；
- R24只完成ABI/owner checkpoint，不证明A/B generation lifecycle。

## Ready queue与真实依赖

1. 立即启动全新只读D41，任务合同：
   `tasks/P5-D41-r05-real-transcript-entry-preflight.md`。
2. 若D41确认入口缺失，root先写最小harness开发合同，使用全新开发Agent实现；direct tests后合流，再由全新I31 owner运行
   一次cheap combined。
3. 只有D41给出唯一精确命令且必要combined PASS后，才用全新R05 Agent运行一次真实A/B transcript。
4. R05 PASS后验证Cargo.lock refresh是否no-op；任何manifest/lock变化必须单独提交并使相应Rust证据失效。
5. 然后运行I02一replica combined与最终R02；PASS后才进入Wave 3的T06–T12/external repo扇出。

当前blocking事实：没有找到可执行的R05真实A/B transcript入口；这是test-infrastructure implementation缺口预警，不是已确认
设计缺口。D41之前不得运行旧single-generation smoke冒充R05。

## 调度约束

- 每个新DAG节点使用全新Agent；原reviewer只复验自己刚提出的同一精确blocker。
- 先审计、批量修复、cheap combined，最后一次真实probe；不得串行试跑完整transcript寻找blocker。
- 当前旧会话受硬线程上限影响，实际上一次只能启动一个新Agent；用户home配置已设
  `max_concurrent_threads_per_session = 20`，新主对话重新确认实际平台槽位。
- 主Agent保持低频工具调用与10分钟等待窗口。
- 不push；跨repo改动分别提交。最终验收前不得操作stable。
