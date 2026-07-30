# P5-F445H I7 P8 D0 HTTP entry 测试权威结果

状态：

```text
PASS
DECISION_REQUIRED = NO
IMPLEMENTATION_UNBLOCKED = YES
```

## 1. Zero-worktree preflight

精确baseline为Skiff
`3a87d37f81a04c249f308b311bd91dcfdf3a8aa3`
（tree `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`）。

只读事实：

- M6的四条失败都直接调用handler/helper，未经过Router/raw HTTP entry，所以没有response stream
  context；
- test-runner已经为每个case生成唯一synthetic service id、精确contract version/deployment与
  ordinary RuntimeAssembly ingress；
- isolated runner已经拥有真实Router/Runtime，Router business HTTP已有service/version精确选择；
- `std.http.request`、`std.http.stream`、inline effect registry、HTTP disconnect/cancel/backpressure
  和父case finalization均已有owner；
- 缺口是把这些能力按self-ingress最短链组合起来，而不是缺一套新的公共HTTP API或协议。

因此权威owner为：

| 事实 | owner |
| --- | --- |
| package/service/ingress长期语义 | `doc/architecture/package-service-contract-deployment.md` |
| 测试源码与effect/finalization语义 | `doc/reference/testing.md` |
| isolated runner编排 | `doc/architecture/test-runner-runtime-isolation.md` |
| 复用普通HTTP client surface | `doc/reference/std-surface.md` |
| 剩余失败证据 | `P5-F445H-I7-M6-aihub-post-g2-diagnostic-result.md` |

## 2. Decision ledger

| 问题 | 决定 | 理由 |
| --- | --- | --- |
| 测试HTTP API | 复用`std.http.request/stream` | 现有类型、effect和stream lifecycle足够 |
| URL | runner提供普通动态business ingress URL | 不猜固定端口，不发明特殊URL |
| 路由 | 自动使用case唯一service/version；Router普通路由 | 现有selector和synthetic identity已闭环 |
| ingress声明 | test service显式`http.yml` + test wrapper | 不自动投影subject ingress |
| registry | exact case deployment共享父registry | entry内部outbound double必须可见 |
| finalize | 仅父case | 子HTTP请求不是第二个test case |
| 并发 | 首版每case一个active self-ingress | 避免registry/finalize竞态，满足当前四条case |
| stream断言 | 完整body或完整协议event | 网络chunk不是业务边界 |
| 安全模型 | isolated runner trust boundary | 不引入生产级session/token/header协议 |

最初候选曾把本能力设计为`std.test.http`、特殊test session与额外内部header。它没有先证明现有
HTTP client、case identity、selector和隔离stack不足，增加了API、状态与生命周期owner，已删除且不得
进入后续任务合同。

## 3. Implementation DAG

```text
D0 authority
├── K  test-runner最短闭环
├── H  Runtime exact-case共享（只在RED证明需要时改production）
└── R  Router普通路由复用审计（默认NO-OP）
       ↓
T  Skiff合流真实HTTP fixture/probe
       ↓
S1 PackageDirect→raw HTTP stream registry闭合
       ↓
I  AIHub四条case迁移
       ↓
X  独立验收
       ↓
J  final hermetic gate
```

K/H/R从同一Skiff baseline预检，写集互不重叠。T只能在K/H/R结果合流后建立GREEN证据。S1先用
concrete Host/raw HTTP gateway与wrapper→`PackageDirect` producer交叉fixture建立稳定RED和registry
identity证据，不能把I观察到的错误文本直接写成已知根因。I使用精确Internals baseline并在S1完成后恢复；
X/J必须在dispatch时记录两个repo的冻结commit/tree。

## 3.1 P8-S refinement ledger

| 问题 | 当前事实与决定 |
| --- | --- |
| 已知根因 | 未知。I的可恢复checkpoint观察到`unknown Stream value`，但现有日志缺少create/lookup两侧registry identity，静态源码也未证明创建了第二个registry |
| package-local stream | 同一request/assembly内wrapper→`PackageDirect` stream应共享当前request已有registry；必须先由S1真实fixture证明当前实现偏离该规则 |
| heap关系 | producer/consumer heap可以不同；item由既有`StreamInternalItem`搬运，不能据此要求共享heap或新增stream bridge |
| service call | 继续使用既有boundary materialization；S1不能把package-local association修复扩张成跨service registry共享 |
| HTTP父子request | stream registry保持隔离；只共享`TestEffectCaseContext`，wire snapshots在HTTP child当前runtime生成handle |
| production修改门槛 | S1必须先记录create/lookup registry identity、request generation与stream id，形成稳定RED；没有RED不得修改production |
| 禁止方案 | 不新增registry、协议、header、schema、compiler、Router、test-runner或测试专用bridge |

## 4. Preflight result

```text
PREFLIGHT_CLASSIFICATION = EXISTING_CAPABILITIES_COMPOSITION
PUBLIC_SURFACE_CHANGE = NO
COMPILER_CHANGE_EXPECTED = NO
STD_CHANGE_EXPECTED = NO
FILE_IR_CHANGE_EXPECTED = NO
ROUTER_PRODUCTION_CHANGE_EXPECTED = NO
```

本result只证明设计与DAG可执行，不是implementation PASS。
