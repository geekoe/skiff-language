# P5-R30：F23D Real Smoke Fourth Reacceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。
DAG节点R30依赖同一exact candidate上的I28 PASS；PASS完成F23D并解除R24。

这是R29失败后的下一次完整探针。它只因D39A/B已重新审计全部剩余生产路径、F28A/B/C批量关闭所有独立finding且I28
combined PASS才获准执行。D39确认无法由便宜证据替代的最小动态范围是跨进程
commit/re-register→真实WS receipt→runtime wire→Event/Result→native direct-send marker→真实cleanup。

必须使用未参与F23/F24/F25/F27/F28、R26/R28/R29/I28的全新只读Agent，在I28同一exact candidate只执行一次：

```bash
node scripts/run-package-service-ecosystem-smoke.mjs --probe skiff-cutover --replicas 1 --checkout "$PWD"
```

不得编辑、提交、修复、重跑或运行combined/full/I16/Host/stable。第一行只给`R30 PASS`或`R30 FAIL`；PASS须给strict
bootstrap/activation/readiness、single WS、Event/Result、native marker与cleanup的有界证据；FAIL给第一错误与F26A/lifecycle
diagnostic且不得重试。证据只对I28 exact HEAD/tree/Cargo.lock及本次隔离环境有效。
