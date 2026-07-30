# P5-D33：Canonical WebSocket Closure Audit Result

状态：complete。D33A–D33C三个全新只读owner在R05 exact candidate
`c277e458dab34305e4b7004d9b08b14ac81a10a7` / tree
`6d39a6e0097ae22b2fcfb413f676e6f3241bbae2`闭合Router lifecycle、Runtime response与真实production evidence；
没有修改、提交、full/Host/I16或stable操作。

## 闭合矩阵

| 跳点 | production事实 | 缺口 / 被遮挡范围 | owner |
| --- | --- | --- | --- |
| server upgrade | server装配真实registry/dispatcher/gateway | 正例注入fake，未走production registry | F23A/F23D |
| registry/dispatch | registry只接受HTTP unary | canonical WS在首帧前必拒绝；gateway还越层直接依赖registry | F23A |
| connect metadata | query/header基本保序 | Cookie固定空；absolute-form URL可覆盖authority | F23D |
| response trust | TS/Rust使用optional bag | accept/reject/HTTP/WS/payload字段可非法组合 | F23A/F23C |
| lifecycle | generic与Assembly各自持有一套 | Assembly缺queue/cancel/backpressure/index/limit/shutdown | F23B/F23D |
| identity/downlink | 正向带service/entry/connection | Runtime未重算identity；sender socket未绑定；负向静默 | F23A/F23C/F23D |
| runtime route | 单请求持有`Arc<ActiveAssemblyRoute>` | B commit替换唯一active，旧A receive必拒绝 | F23E/F03C |
| Router pin/release | gateway保存A header与runtime socket | 无跨层release/ack、runtime disconnect/drain闭环 | F23E/F03B |
| projector | 唯一service boundary dispatcher | canonical projector复制在legacy巨型模块 | F23C |
| real evidence | ABI/wire/eval分段PASS | 无真实Router+Runtime A→B→A/A→B→unary B→close transcript | F03B/F03C后R05 |

现有R05合同与F03B/F03C形成执行环：R05要求真实old-generation pin，但该production owner明确属于被R05锁住的
F03B/F03C。该环是阶段DAG错误，不是业务设计缺口。权威语义已有唯一答案：connect成功后按完整tuple隐式acquire，
close/policy/runtime disconnect显式幂等release；Router与Rust trust boundary分别重算冻结identity；单向
`connection.send`的跨service/entry/sender错误是protocol violation，合法closed race必须产生结构化delivery-miss，
不增加source同步ACK。无需用户设计决策，也不改变四对象或std ABI。

## 批量修复DAG

```text
F23A Router trust/dispatch ─┐
F23B shared lifecycle core ├─► F23D Assembly convergence ─► combined ─► R24 ABI/owner checkpoint
F23C Runtime response      ┘                                             │
                                                                        ▼
                                                     F23E generation lifecycle wire
                                                                        │
                                                        ┌───────────────┴───────────────┐
                                                        ▼                               ▼
                                                     F03B Router                     F03C Runtime
                                                        └───────────────┬───────────────┘
                                                                        ▼
                                                        combined + narrow acceptance
                                                                        ▼
                                                         R05 real production acceptance
                                                                        ▼
                                                                       I02
```

F23A/B/C写入边界互不重叠，可并行；F23D只在三者合流后消费接口。R24只接收F05 ABI、owner与单generation真实组件路径，
不冒充A/B lifecycle verdict。F23E独占shared TS/Rust lifecycle wire，随后F03B/F03C并行消费且不得回改shared seam。
最终R05使用全新reviewer，只运行一次隔离Router+Runtime transcript和必要窄负例。

当前会话虽已把新会话配置上限设为20，但host仍固定4个总槽且两个历史completed thread占位，实际只能滚动运行一个
新Agent；这是平台调度限制，不是DAG依赖。若新会话释放槽位，应立即并行F23A/B/C和后续F03B/F03C。
