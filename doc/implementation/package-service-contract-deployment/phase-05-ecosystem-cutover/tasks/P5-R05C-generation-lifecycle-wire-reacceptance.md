# P5-R05C：Generation Lifecycle Wire Reacceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第8、10、11条，§7、§12及§14。

DAG节点R05C，依赖I35关闭。F45B/C/D修改shared runtime control wire与Router/Runtime connection consumers，使R05B
证据失效；R05C是新稳定周期唯一generation lifecycle reacceptance，不重复静态审计或其它gate。

exact production candidate为commit
`95296242921cf26dfe961a735f652a84caf249b4`、tree
`2768f0822ed68ad511723442a45604e18a32c115`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新独立只读Agent只运行一次：

```bash
node scripts/run-package-service-generation-lifecycle-smoke.mjs \
  --probe r05-generation-lifecycle \
  --replicas 1 \
  --checkout "$PWD"
```

必须重建A旧连接×2、B WS/unary、canonical SKPV decode、两次exact release ACK、pin
`0→1→2→1→0`、最终in-flight 0/pending null及cleanup。禁止试跑、重试、旧smoke、full Host/I16、编辑、提交、
stable或额外instance。第一行`R05C PASS`或`R05C FAIL`；FAIL给最小反例/唯一owner并停止。PASS只解除I02。
