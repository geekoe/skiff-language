# P5-I33：R05 Tail Closure Combined

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点I33，依赖F42/F43/F44全部合流到production commit
`c59b4baf9752147cc49c141d89642d8b7f5aa507`、tree
`08051c65166eec977748b5b58c4636d26cb5eff4`，Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。I33是第三次真实probe前唯一cheap combined owner，不作
R05/R02/Phase verdict。

全新只读Agent在exact合流状态各运行一次：

```bash
node --test \
  scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router exec vitest run \
  tests/websocket-generation-lifecycle-router.test.ts \
  tests/assembly-runtime-endpoint.test.ts
git diff --check
```

combined必须确认local HTTP success server返回canonical SKPV bytes而非JSON、consumer使用唯一shared codec、ACK/pin
尾段完整；Router direct必须闭合matching ACK/reject/timeout/disconnect/new connection及health snapshot。随后静态反搜：

- generation consumer不存在`JSON.parse(response.body)`或JSON success fake；
- Router+scripts不存在第二个`SKPV` parser；
- exact ACK序列及字段名与F43 health一致。

禁止编辑、提交、修复、真实transcript、旧smoke、fixture combined、instance/stable或完整gate。FAIL只归类并返回唯一
owner，不重跑。PASS只解除全新R05B Agent在冻结candidate上运行第三次且仅一次真实transcript。

F42/F43/F44表面、Router lifecycle/health、Node HTTP/Buffer、Cargo.lock、测试源码或checkout source变化使I33失效。
