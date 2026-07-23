# P5-R05：Canonical WebSocket Ingress Acceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点R05，依赖R24 PASS、F23E、F03B/F03C/F30A、I30 PASS，以及D41冻结的真实transcript入口；若D41确认缺harness，
还依赖其后续实现与cheap combined PASS。风险高，验收分组为canonical WebSocket production generation lifecycle。

必须使用未参与F05/F23/F03B/F03C/F30A、旧R05或D41/harness实现的全新独立只读Agent。输入为交接后冻结的exact clean
integration commit/tree/Cargo.lock、D05/F05合同、R02A seam、R24/F23E/F03B/C/F30A/I30证据及D41冻结命令。不得编辑、
提交、修复或给R02最终verdict。旧R05在`c277e45`永久保持FAIL，本合同是production lifecycle闭合后的全新最终验收批次。

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
registry/dispatcher/direct-send emitter，且不得重跑full Host/I16/stable。精确命令必须由D41及必要harness任务提前写入合同/
handoff，验收Agent不得临时拼装或先试跑。第一行只给`R05 PASS`或`R05 FAIL`。PASS与已通过的I30/必要harness combined共同
解锁Cargo.lock no-op/refresh验证与I02；FAIL给最小production反例、失效证据与唯一owner，不重试。

证据只对最终派发的exact candidate与本次隔离环境有效。Router/Runtime lifecycle、store provisioning、fixture/transcript、
shared wire、Cargo.lock或环境来源变化会使其失效。
