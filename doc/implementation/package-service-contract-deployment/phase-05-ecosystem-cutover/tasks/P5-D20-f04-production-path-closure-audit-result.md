# P5-D20：F04 Production Path Closure Audit Result

## 结论与候选

`D20 CLOSED`，不等于F04或阶段PASS。审计锚定
`e786671cd7d28e7efe911703cc5b2f1f0ff51ab1` / tree
`caadc57696c83e1f28dd00fa282d812d13c5c561` / `Cargo.lock`
`f3ce5457138c58aec4c84abda431afa96013e3fd`，工作树clean。

同一真实入口`node scripts/run-skiff-tests.mjs`已连续暴露超过两个新blocker，跨层收敛熔断维持：repair wave、I16
combined与窄验收完成前禁止完整source-suite/Host。D20A–D20F及D20D/E补充预审均为全新只读Agent；没有Agent修改
候选或给阶段verdict。全部发现都落在既有架构语义内，`DESIGN DECISION REQUIRED: none`。

## 闭合矩阵

| jump | production owner | input → output | 已有正/反证据 | unseen / blocker | repair / cheap closure |
| --- | --- | --- | --- | --- | --- |
| 1 source registry/default root | `skiff-source-test-registry.mjs`、source suite | module-owned checkout root → exact `[{id:std,root:std}]` | command-double锁定唯一resolver、duplicate/escape拒绝 | F16C后exact registry已复核，无新blocker | I16结构digest |
| 2 isolated config/env/ports/root | isolated runtime scripts | root/base env → leased ports、config、runner env | 端口/root/cleanup negatives | inherited platform env、relative Cargo target双cwd、outer readiness fail-open | F18C |
| 3 bootstrap generation 0 | smoke fixture + isolated instance | canonical roots/env → empty assembly + committed gen0 | fixture parser/receipt tests | production transport已闭；受jump2 provenance影响 | F18C direct tests |
| 4 compiler platform context/prelude | `CompilerPlatformSources`、PreludeRegistry、authoring/runner | absolute root → canonical manifests/sources/context | F16A/B/C typed negatives与golden identity | Prelude raw枚举跨根；different-root guard晚于source IO | F18A、F18B |
| 5 std overlay/artifact | canonical package/overlay/store | std/tests/contracts → PackageArtifact/test assembly | 12 pass/1 ignored；overlay/base negatives | compiler integration common helpers仍是旧签名，18 targets不能编译 | F18H；F18A/B merge probe |
| 6 Host fixture/base assembly | Host preparer + fixture writer | checked-in roots → packages/deployments/base receipt | fixture/receipt/schema direct tests | helper新增第8参使Clippy失败；真实negative result缺证据 | F18I、F18G/H18 |
| 7 supervisor/Router/Runtime startup | `skiff-instance`、Router server、Runtime driver | config/FD/PID/child → single endpoint/session | F17 factory 20轮、config tests | `stopped:false`被当成功；unsupervised acquisition无统一cleanup | F18E |
| 8 committed recovery/registration/readiness | Runtime lifecycle/admission、Router registry、runner readiness | durable tuple/control frames → healthy exact replica | R11/R15核心direct tests | outer readiness可接受错ID/env/缺connected；当前file store跨实例不安全 | F18C、F18D |
| 9 prepare/admit/commit | Router coordinator/File store/Runtime admission | expected generation + frozen participants → durable commit/abort | Router/Runtime focused tests | File store mutex仅实例内，双实例20/20双成功last-write-wins | F18D |
| 10 Host ingress request | test runner + HTTP gateway | Host/body → canonical nested request | Router unary 4/4，legacy selector fail closed | 当前positive完整Host禁跑；下游negative组合未直证 | F18G/H18、G16 |
| 11 dispatch/decode/eval | registry/dispatcher/runtime wire/assembly | exact active tuple + binary frame → eval response | R13组件与当前direct tests | false assertion完整错误链未直证 | F18G/H18 |
| 12 boundary/helper mutation | linked image/eval/boundary materialization | same-heap helper + detached service → exact result | linked/eval/fixture direct tests | 当前candidate完整组合被上游遮挡 | H18 negative、G16 positive |
| 13 response/result assertion | Runtime mapper、Router gateway、runner | result/error → 2xx PASS或non-2xx FAIL | 分段success/error/once证据 | 缺真实false assert→500→exit1/no retry | F18G/H18 |
| 14 cleanup/provenance | gate ownership、isolated runtime、supervisor lifecycle | worktrees/target/PID/FD/ports → ledger + absence | F17及gate command-double | registry partial add、foreign path/task-root删除、ledger覆盖、PID/process leak | F18E、F18F |

## 合并后的独立blocker

1. **F18A compiler trust**：PreludeRegistry重复枚举/读取official `.skiff`，绕过canonical containment；真实root-outside
   symlink会被编译并发布artifact。
2. **F18B compiler ordering**：authoring与runner在`DifferentPlatformRoot`前读取package manifest/source。
3. **F18C isolated boundary**：platform test env继承、relative Cargo target双解释、readiness身份/environment/connected
   fail-open属于同一写owner。
4. **F18D durable CAS**：File activation store锁只在实例内；同root双实例并发不满足CAS。File/Memory reducer分歧和
   persistence failpoints一并在该owner关闭。
5. **F18E process lifecycle**：`stopped:false`仍清PID/允许restart；unsupervised PID/handle acquisition失败不经过单一
   lifecycle，可能泄漏group/FD或误删PID。
6. **F18F gate ownership**：Git registry/path/task-root/ledger没有nonce+identity no-clobber owner，失败cleanup可能遗留
   registry或删除/覆盖foreign资源。
7. **F18G result evidence**：缺一个不运行完整suite的真实Host false assertion focused-negative harness。
8. **F18H verify transport**：compiler test common helper遗留三处F16A旧签名，18个integration targets均无法编译；
   production `--lib --bins`不受影响，但最终verify必失败。
9. **F18I candidate hygiene**：F16C使Host fixture helper达到8参，当前candidate的`clippy::too_many_arguments`为R15
   强制gate blocker。

R15独立复验另见`P5-R15-readiness-reacceptance-result.md`：四个历史readiness blocker及request-once均关闭，但因F18I
返回FAIL。旧phase-plan中的`R15 PASS@e3a0d78`没有可恢复的独立ledger，不再作为PASS证据。

## 非阻断债务与证据边界

- `skiff-instance.mjs`、Router registry/endpoint仍较大；F18D/E只抽当前新owner，不做无关全面重构。
- canonical artifact写入与manifest compile编排存在跨compiler/runner重复，但不是本波新公共抽象；不得借repair反转依赖。
- 当前Router/Runtime/request/eval历史组件证据未被F16改动的代码面可复用，但不能替代G16当前candidate完整positive结果。
- 完整Host运行计数仍为0；D20的临时/组件probe不计入G16，也没有操作stable。

## 批量repair DAG

```text
docs checkpoint
├─ F18A Prelude containment
├─ F18B pre-read context guard
├─ F18C isolated provenance/readiness
├─ F18D Router file CAS
├─ F18E supervisor/process lifecycle
├─ F18F gate resource ownership
├─ F18G Host negative harness
├─ F18H compiler test-only transport
├─ F18I Host fixture Clippy closure
└─ F18J authoring pre-store guard（R18A在首次combined后发现）
       ↓ 全部clean commits合流；无在途写入
I16 replacement cheap combined probe
       ↓
R15B + compiler trust + Router CAS + resource lifecycle narrow reviews
       └─ H18 focused-negative真实执行
              ↓
new R16 → G16 one positive full Host → new F04 receive
```

前九个开发节点写集互不重叠，可从同一docs checkpoint并行；F18J是R18A独立验收发现的同owner后继窄修复。受实际运行时限制当前最多同时活跃两个root child，滚动
调度不构成语义依赖。任何节点需要越过自己的exclusive write set、修改manifest/lock或公共设计时必须
`TASK_NOT_EXECUTABLE`。
