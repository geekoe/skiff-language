# P5-F375 Registry generation revalidation

状态：Ready（真实发布与隔离运行时验收；默认只读consumer验证）。

## 直接父节点

- `P5-F365-host-http-gateway-admission-wire-result.md`
- `P5-F369-registry-error-payload-marker-cleanup.md`
- `P5-F350-external-ingress-ecosystem-migration-audit-result.md`第10.2节

父节点已经冻结：

- Registry没有external ingress，source无需迁移；
- Registry的20个普通service-call operation必须保持`20 -> 20`；
- F369已经从两个Registry错误类型删除过时`ErrorPayload` marker，production commit
  `fcc1d87f4a668521020e24fed38575a27535508d`，已合入skiff-packages integration；
- Host canonical HTTP gateway admission已经完成。本节点只关闭此前延后的新generation Registry真实验收。

## Checkpoint与范围

- Skiff toolchain：以包含本任务的`/Users/geek/workspace/skiff-phase-05-integration`为准；
- skiff-packages consumer：
  `/Users/geek/workspace/skiff-packages-phase-05-integration`，
  checkpoint `0ab4e7628b0a6aa90961c1485d2e58634b902676`；
- 本任务worktree只用于写result文档；skiff-packages integration默认只读。

不得操作stable/live，不得修改Registry API、manifest、contract surface或其它package。若真实发布/运行暴露
production问题，只做精确定位并返回`TASK_SCOPE_EXPANDED`；不要在验证节点跨仓库修复。

## 必须验证

1. 在fresh temporary artifact/store中bootstrap canonical std并真实发布Registry service package，不复用
   旧receipt。
2. 验证ServiceContract：
   - 精确20个operation；
   - 四类record各五个：
     `packageArtifact`、`serviceContract`、`serviceDeployment`、`runtimeAssembly`的
     Put/Read/PointerRead/PointerCas/PointerHistory；
   - 全部Available；
   - 无gateway ingress entry。
3. 使用隔离test/runtime路径验证新版PackageArtifact、ServiceContract、ServiceDeployment和RuntimeAssembly
   identity摘要可以immutable put/read，并完成pointer CAS/history语义；不得只做source shape检查。
4. 运行并记录非零测试：

```bash
cd /Users/geek/workspace/skiff-packages-phase-05-integration
npm run test:registry
npm test
```

5. 反搜确认：
   - skiff-packages production `.skiff`中`implements ErrorPayload`为零；
   - Registry operation仍为20；
   - 其它official package没有意外生成ServiceContract。

## 交付

在本任务Skiff worktree写
`P5-F375-registry-generation-revalidation-result.md`，记录：

- Skiff与skiff-packages exact commit/tree；
- fresh Registry package/service/deployment/assembly receipt或测试实际可观测的对应identity；
- 20-operation清单/计数、零ingress证据；
- immutable record及pointer路径的非零测试；
- `npm run test:registry`与`npm test`计数。

result一个本地commit，worktree clean；不merge/rebase/push。新Agent执行，不派子Agent。
