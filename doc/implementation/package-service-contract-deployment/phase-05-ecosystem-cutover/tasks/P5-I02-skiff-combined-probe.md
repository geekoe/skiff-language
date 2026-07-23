# P5-I02：Skiff Consumer Combined Probe

## 角色与输入

主integration owner在T02–T05、F03A–F03C全部合入Skiff integration、共享lock串行刷新、工作树clean且无在途
写入后执行；不是开发、gate或独立验收owner。输入是R01/R02A checkpoint、task commits/ledgers、R02预审
矩阵与合流后的exact commit/tree。
不得修改source/tests/fixture/config、创建commit或操作stable。

本批次exact production candidate为commit
`c59b4baf9752147cc49c141d89642d8b7f5aa507`、tree
`08051c65166eec977748b5b58c4636d26cb5eff4`、Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。I34已证明shared-lock no-op及locked compiler PASS。

## 唯一命令与完成态

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-i02-cargo.XXXXXX)"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
  node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1
git diff --check
```

脚本必须使用temporary artifact/runtime homes和动态端口，从canonical authoring写四对象，执行router
prepare → runtime staged ACK → commit → register，再由Host ingress到provider最终业务结果。随后以tampered
candidate触发reject/abort，断言committed tuple与旧request结果不变、pending/staged资源归零、request path
artifact I/O为零。不得注册stable watch或调用stable reload。

同一唯一运行还必须覆盖R02预审的真实互操作缺口：Router幂等bootstrap empty generation 0；Runtime cold
startup读取并注册committed state；capabilities握手不掉线；binary assembly frames双向；至少一次actor/spawn
control得到typed response；先激活A并打开server stream或WS，再激活B，证明新unary到B且旧连接/stream继续
pin A直至自然结束。request.start使用canonical nested assembly routing，Rust/TS无unknown-field漂移。

其中A/B generation lifecycle已由同一exact candidate上的R05B唯一真实transcript建立，不在I02重复；I02本命令只补齐
authoring→activation transaction→Host最终结果与tampered candidate reject/abort rollback。R05B证据保持有效是I02
PASS前置，不能用I02旧single-generation smoke替代或重跑。

输出exact commit/tree、命令exit/耗时、activationId/generation/assembly/replica、最终结果与rollback
断言。PASS才可提交R02；FAIL退回受影响owner，修复合流后只重跑本probe与失效的聚焦证据。
