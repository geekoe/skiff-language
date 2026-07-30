# P5-R05A：Canonical WebSocket Ingress Reacceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点R05A，依赖R05精确FAIL、F41A合流及I32 PASS。风险高，是R05新稳定周期的独立reacceptance；必须使用未参与
D41/F41/F41A/I31/I32或旧R05的全新只读Agent。不得编辑、提交、修复或给R02/Phase verdict。

exact production candidate为commit
`8c832b44a49b31da393064ab2c6c7d432db70274`、tree
`9f55ccc9afc87b4d3d350e3dd416f5150149e343`、Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。后续只允许Phase 5证据文档提交，派发前须证明无production diff。

R05未受F41A影响的静态设计证据与I31 author/store证据继续有效；I32已在exact candidate上7/7 PASS。R05A只复验
失效的动态边界，必须在真实隔离Router+Runtime child上运行一次且仅一次：

```bash
node scripts/run-package-service-generation-lifecycle-smoke.mjs \
  --probe r05-generation-lifecycle \
  --replicas 1 \
  --checkout "$PWD"
```

必须观察完整固定transcript：

- A connect，activate B后A receive两次仍为A marker；
- B connect/receive及unary均为B marker；
- close B后pin回1，close A后pin/in-flight均为0且无pending activation；
- 使用正常source/compiler/store、真实production ingress及isolated owner cleanup，无fake、retry、fallback、
  artifact patch/re-sign或stable操作。

禁止试跑、重试、旧smoke、full Host/I16、fixture combined或额外instance。第一行只给`R05A PASS`或
`R05A FAIL`。PASS与仍有效证据共同关闭R05并解锁Cargo.lock no-op/refresh验证；FAIL给最小production反例、
diagnostic、失效证据和唯一owner，不重试。

Router/Runtime lifecycle、F23E wire、store/provisioning、isolated owner、fixture/transcript、activation/health、
Cargo.lock、Node HTTP行为或checkout/environment source变化会使R05A失效。
