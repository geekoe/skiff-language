# Bytecode VM Baseline Benchmark Manifest（Phase 0）

状态：planned（本文件随 Phase 0 冻结，Phase 9 执行前不得修改 workload/口径/阈值，见
[`../phases/phase-9-release.md`](../phases/phase-9-release.md) §3.3）。

本 manifest 满足 [`phase-0-baseline-live.md`](../phases/phase-0-baseline-live.md) 交付物 5：
固定机器、release/debug profile、workload 清单、采样窗口、统计口径与 baseline commit。

**重要声明**：Phase 0 的 baseline 结果由 tree evaluator（旧 async 树遍历求值器）产生，
**不构成 bytecode VM 的证据**。它只固定"迁移前"的性能事实，供 Phase 9 与 VM 结果对比；
任何把 baseline 当作 VM 证据的用法都是错误。

## 1. 机器信息

| 项 | 值 |
| --- | --- |
| 平台 | darwin / arm64（`uname -m` = arm64） |
| 芯片 | Apple M4 Pro（`machdep.cpu.brand_string`） |
| 核数 | 14（`hw.ncpu`） |
| 内存 | 24 GiB（`hw.memsize` = 25769803776 bytes） |
| 运行环境 | 本机 `/Users/geek/workspace` 共享 cargo target（`.skiff-cargo-target`）；benchmark 期间不并发跑 cargo |

机器漂移控制：结果记录时必须同时记录 CPU 频率档位约束（如有）、并发负载（benchmark 前后
`uptime`）、内存压力与 thermal throttling 迹象；无法证明机器状态可比时，结果不作为对比证据。

## 2. Profile

| Profile | 用途 | 说明 |
| --- | --- | --- |
| `debug` | 功能验证与迭代正确性 | 只用于 Phase 1–8 的功能 gate；不进性能对比。 |
| `release` | 性能对比（Phase 9 门槛） | 唯一进入 baseline/candidate 性能对比的 profile；必须记录 binary SHA。 |

规则：任何性能结论必须来自 `release` 构建；`debug` 的计时、alloc 或 RSS 数据不得写入
baseline 或候选结果。Baseline 与候选必须使用同一 profile、同一机器、同一统计口径。

## 3. Workload 清单（从 phase-9 §3.3 向前映射，Phase 0 登记）

以下 workload 名称/口径/采样窗口为**占位登记**：Phase 0 必须为每个 workload 落成可复现的
harness 入口（含输入 fixture 与固定 seed），Phase 9 只执行不新增。workload 映射自
`phases/phase-9-release.md` §3.3 的 release benchmark 清单。

| # | Workload | 口径（placeholder） | 采样窗口（placeholder） | Phase 9 对应 |
| --- | --- | --- | --- | --- |
| W01 | `pure_loop` | 纯表达式/控制流循环，固定迭代数 | 每样本固定迭代，≥5 样本 | pure loop |
| W02 | `deep_local_calls` | 深 local 同步调用链（non-tail） | 固定深度 × 固定迭代 | deep local/non-tail calls |
| W03 | `tail_calls` | 深尾调用链（tree 侧用 `Flow::TailCall`） | 固定 hop 数 × 固定迭代 | tail calls |
| W04 | `ready_unary` | sync/ready unary request 全链开销 | 固定并发 × 固定请求数 | Ready unary request |
| W05 | `dense_record` | dense record 构造/投影/字段访问 | 固定大小 × 固定迭代 | dense record |
| W06 | `unique_array_map` | unique Array push / Map put 与 mutation | 固定元素数 × 固定迭代 | unique Array/Map |
| W07 | `nested_cow` | shared snapshot 后沿嵌套 path COW mutation | 固定深度/宽度 × 固定迭代 | nested COW |
| W08 | `materialize_json_db` | string concat、JSON encode/decode、DB 值物化 | 固定 payload 大小 × 固定迭代 | string/JSON/DB materialization |
| W09 | `interface_dispatch` | local / remote / callback 三 carrier dispatch | 固定调用数 × 固定迭代 | local/remote/callback dispatch |
| W10 | `sync_child` | 同步完成的跨 owner child（InProcessBoundary） | 固定调用数 × 固定迭代 | sync child |
| W11 | `pending_park_resume` | actual Pending park/resume 与 pending cleanup | 固定挂起数 × 固定迭代 | actual Pending park/resume、pending cleanup |
| W12 | `stream_backpressure` | stream producer/consumer 与 backpressure | 固定 item 数 × 固定 buffer | stream backpressure |
| W13 | `long_request_gc` | allocation-heavy 长 request / GC pressure | 固定分配量 × 固定迭代 | allocation-heavy long request/GC |
| W14 | `actor_segment` | Actor 同步段与 suspension（segment lease） | 固定方法数 × 固定迭代 | Actor synchronous segment and suspension |
| W15 | `agine_chat_smoke` | 真实 Agine LLM SSE reducer/chat smoke | 固定 prompt × 固定轮数 | real Agine chat |
| W16 | `host_tools_profiling` | strict full host-tools profiling（sample 非空校验） | 固定工具调用 × 固定轮数 | strict host-tools profiling |

W16 运行约束：只跑正常流程。同一候选上 host-tools 正常流程各跑一次，不多跑角度/注入的
CLI 级对话；注入与多角度验证由 host-tools 单元测试（`client/e2e/host-tools-strict.test.mjs`）
覆盖同一 strict 断言路径。host-tools 单实例串行：每个实例 spawn 独立 agine-host 并以固定
host name 注册 provider，同一 gateway 上并行会互相干扰，且真实对话打外部 LLM。

Phase 0 注册要求：每个 workload 至少固定输入 fixture（文件路径 + content hash）、固定迭代/样本
计划与可复现 seed；harness 必须能够输出 per-sample 原始数据（不只输出聚合值）。

## 4. 统计口径

| 项 | 规则 |
| --- | --- |
| warmup | 每 workload 先跑 ≥1 轮 warmup，不计入样本；warmup 轮数固定并记录。 |
| 样本数 | 每 workload 每配置 ≥5 个有效样本（Phase 0 可加，不可减）；报告必须给出样本数。 |
| 聚合 | 报告 median + p95 + p99 分位数；成对对比时报告置信区间（默认 95% CI，方法与实现固定）；不使用"最快一次"作为结论。 |
| 指标名 | `cpu_time`（进程 CPU 时间，不混入 wall 偏差）、`wall_time`（仅上下文标注用）、`rss_peak`（峰值 RSS）、`allocated_bytes` / `allocated_objects`（分配总量，tree 侧经 profiling/`allocation` 采样）、`gc_cycles` / `gc_pause`（若被测面有 GC，Phase 0 为 0 或 N/A）。 |
| 异常处理 | 样本中出现异常值（外部负载、throttle、错误）时：保留原始记录、标注剔除理由，并报告剔除数；不允许静默丢弃后只报剩余样本。 |
| binary SHA | 每个结果必须附带被测 binary 的 SHA（`skiff`/`router`/`runtime` 各一份）与源码 commit；SHA 缺失的结果无效。 |

Phase 0 还需固定：计时 API（`clock_gettime`/`Instant` 语义）、进程亲和与 OMP/线程配置（本机为
单进程 benchmark，禁止并行跑多个 benchmark 实例）、以及"收集 RSS 的时机"（request 完成后
terminal 前）。

## 5. Baseline commit

| 仓库 | 路径 | baseline commit（Phase 0 冻结日） |
| --- | --- | --- |
| skiff（本仓库） | `/Users/geek/workspace/skiff` | `7e2e38724fb75544123f133f869c29f89cd2d3da`（main HEAD，`docs(implementation): define bytecode VM phase gates`） |
| internals/agine | `/Users/geek/workspace/internals/agine` | `3db542a62962e160ddd7f8bb84b3ea025c9a7132` |
| skiff-packages | `/Users/geek/workspace/skiff-packages` | `db4ddd9e05936b6fa8beff42ed242c8a73f08de3` |

规则：

- baseline 语义 = "tree evaluator 在当前 main 上的性能"，由 tree evaluator 产生；VM 上线后
  的候选必须与同一 baseline commit 对比（Phase 9 用同一三仓 commit 复现 baseline 环境）。
- 修改上述任意 commit 后必须重新冻结 baseline（新 evidence epoch），旧 baseline 只作参考不作
  门槛。
- baseline 结果归档到 `phases/results/`（phase-0 的 result 目录约定），每个归档包含机器信息、
  profile、workload 参数、原始样本、聚合值、binary SHA 与三仓 commit。
- baseline 不承诺任何"通过/失败"结论；Phase 9 的门槛阈值在 Phase 0 预注册（单独评审输入），
  阈值不能在看到结果后就地放宽。

## 6. 声明

- **baseline 由 tree evaluator 产生，不构成 VM 证据。**
- baseline 只回答"迁移前热路径是什么水平"，不回答"VM 是否满足语义"；语义证据来自
  ledger + 各 phase 的 focused tests 与 Live gate。
- 本 manifest 的修改属于评审输入；冻结后任何 workload/口径/统计变更必须开启新的 evidence
  epoch（phase-0 §7 Handoff）。
