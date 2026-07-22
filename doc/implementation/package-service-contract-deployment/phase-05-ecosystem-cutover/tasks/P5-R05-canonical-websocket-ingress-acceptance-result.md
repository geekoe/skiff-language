# P5-R05：Canonical WebSocket Ingress Acceptance Result

`R05 FAIL`

- candidate：`c277e458dab34305e4b7004d9b08b14ac81a10a7`
- tree：`6d39a6e0097ae22b2fcfb413f676e6f3241bbae2`
- `Cargo.lock` blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`
- tracked状态前后不变；唯一untracked为`.p5-i16-combined-ledger.json`；未编辑、提交或操作stable。

blocking findings：

1. Assembly gateway复制generic gateway的connection/business index、receive dispatch、policy、direct send与close，成为
   第二套且不完整的lifecycle owner；缺少单飞队列、drain/cancel、backpressure、连接上限和bounded shutdown。
2. canonical connect把真实Cookie固定物化为`[]`，production请求携带`Cookie: sid=A`时业务仍观察为空。
3. canonical response projector被追加到千行legacy `websocket_adapter.rs`并复制accept/reject/payload逻辑，不满足
   projector与模块职责唯一性。
4. A/B正例使用fake registry、fake dispatcher和伪runtime connection，只证明Router header/snapshot；没有真实Runtime
   marker、new unary B或自然close证据。

compiler normal source、projection `2/2`、deployment `1/1`、runtime transport `6/6`、eval `9/9`、request `4/4`、
artifact无rewrite smoke、Router type-check、指定三文件`78/78`、wire corpus与diff-check均PASS。冻结pnpm命令误跑全部
Router suite后的5个legacy fixture失败在父树已存在且归D23/T06，不是F05 regression；它们不改变上述production blockers。

同一路径在一次验收中暴露第二个新blocker，触发D33跨层收敛熔断。R05保持FAIL，不解锁F03B/F03C。
