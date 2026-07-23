# P5-R05B：Canonical WebSocket Ingress Final Reacceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点R05B，依赖D42闭合审计、F42/F43/F44合流及I33 PASS。它是熔断后允许的第三次完整probe，必须使用未参与
D42/F42/F43/F44/I33或旧R05/R05A的全新独立只读Agent。不得编辑、提交、修复或给R02/Phase verdict。

exact production candidate为commit
`c59b4baf9752147cc49c141d89642d8b7f5aa507`、tree
`08051c65166eec977748b5b58c4636d26cb5eff4`、Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`；后续仅允许Phase 5证据文档提交，派发前须证明无production diff。
I31 author/store与未受影响静态设计证据仍有效；I33已在exact candidate PASS。

只运行一次且仅一次：

```bash
node scripts/run-package-service-generation-lifecycle-smoke.mjs \
  --probe r05-generation-lifecycle \
  --replicas 1 \
  --checkout "$PWD"
```

必须观察完整真实隔离transcript：

- A connect；activate B后A receive×2仍为A marker；
- B connect/receive为B marker；B unary production SKPV decode后为B marker；
- close B收到exact release ACK后pin回1；
- close A收到第二个exact ACK后pin/in-flight归0且pending activation为空；
- 正常source/compiler/store、真实Router/Runtime ingress及isolated owner cleanup，无fake、retry、fallback、
  patch/re-sign或stable操作。

禁止试跑、重试、旧smoke、full Host/I16、fixture combined或额外instance。第一行只给`R05B PASS`或
`R05B FAIL`。PASS与仍有效证据共同关闭R05并解锁Cargo.lock no-op/refresh验证；FAIL必须返回所有已执行跳点、
最小反例/diagnostic、失效证据与唯一owner，不重试，并再次停止完整probe。

Router/Runtime lifecycle、shared codec/wire、store/provisioning、isolated owner、fixtures/transcript、
activation/health、Cargo.lock、Node HTTP/Buffer或checkout/environment source变化会使R05B失效。
