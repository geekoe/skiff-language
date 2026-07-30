# P5-F44：R05 Raw Decode and Tail Oracle

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点F44，依赖F42与F43合流。目标是让generation lifecycle harness消费F42唯一codec owner，并用F43 exact release
ACK diagnostic闭合B unary到最终A release/drain；完成后只解除I33。

独占写入：

- `scripts/lib/package-service-generation-lifecycle-smoke-real.mjs`
- `scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs`
- generation lifecycle real/oracle/lifecycle direct tests。

要求：

- HTTP 200保留bounded raw Buffer，超过512 bytes fail closed；只有non-200 diagnostic将bytes转UTF-8并脱敏；
- 仅用F42 shared owner按固定fixture return schema`{type:"string"}` decode，再断言B marker；
- JSON 200必须成为missing-magic负例，禁止fake回归或复制`SKPV` parser；
- health顺序固定断言ACK count `0→0→0→1→2`、pin `0→1→2→1→0`，每步`inFlight=0`且pending null；
- 补canonical raw success、truncated body、marker mismatch、ACK/pin/inFlight/pending负例，以及decode primary error不被
  finally cleanup failure替换。

开发owner只运行：

```bash
node --check \
  scripts/lib/package-service-generation-lifecycle-smoke-real.mjs \
  scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs
node --test \
  scripts/tests/package-service-generation-lifecycle-smoke-oracle.test.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs
```

禁止修改F42 codec、Router/Runtime、fixtures、compiler/store、release/activation/四对象或公共ABI；禁止真实transcript、
instance/stable/完整gate。独立worktree/branch从F42/F43合流checkpoint创建，5分钟内修改，否则返回
`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。

I33由全新owner在全部合流后运行合同另行冻结的combined；I33 PASS前不得第三次真实probe。
