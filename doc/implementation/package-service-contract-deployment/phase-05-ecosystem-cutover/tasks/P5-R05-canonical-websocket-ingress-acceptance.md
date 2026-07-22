# P5-R05：Canonical WebSocket Ingress Acceptance

未参与F05/F23/F03B/F03C实现及旧R05失败验收的全新独立只读Agent。输入为R24 PASS、F23E、F03B/F03C exact
clean integration commit/tree、D05/F05合同、R02A PASS seam与合流后聚焦证据；不得编辑、提交、修复或给R02最终
verdict。旧R05在`c277e45`永久保持FAIL，本合同是production lifecycle闭合后的全新最终验收批次。

必验：

- 正常source/compiler/deployment路径产生typed unified WS ABI，四对象schema无变化、artifact无patch/re-sign；
- TS/Rust `websocket.ingressEvent`接受集合、payload segment/context presence/default normalization一致；legacy
  adapter不因新enum放宽；
- runtime只经pinned boundary descriptor与`dispatch_in_process_boundary`，connect/receive返回规则fail closed；
- production direct connection send可观察旧A连接在B激活后仍执行A，新连接/unary使用B；无ambient state/test hook；
- Router entry/gateway identity稳定且隔离service/entry，business index/policy/close/drain owner唯一；
- `extra-review`检查std/compiler/wire/runtime/Router间无重复ABI parser、第二dispatcher或巨型混合职责。

必须在真实隔离Router+Runtime child上只运行一次
`A connect → activate B → A receive×2 → B connect/receive → unary B → close/release/drain` transcript；禁止fake
registry/dispatcher/direct-send emitter，且不得重跑full Host/I16/stable。第一行只给`R05 PASS`或`R05 FAIL`。
PASS与一次cheap combined共同解锁I02；FAIL给最小production反例、失效证据与唯一owner。
