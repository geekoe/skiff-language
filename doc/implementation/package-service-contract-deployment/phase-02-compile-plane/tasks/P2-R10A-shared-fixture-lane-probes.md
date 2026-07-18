# P2-R10A：Shared-fixture Lane Contract Probes

状态：fan-out gate；依赖 R10，R10B/R10C/R10D 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Package 编译”
“ServiceContract 编译”“Fail-closed 条件”。

## 目标与 ownership

- 用一个小型 representative integration target 证明 R10 shared fixture 足以支持三条 consumer lane，再冻结 API。
- 独占该 probe target、对应 `compiler/Cargo.toml` entry，以及 probe 暴露不足时对
  `compiler/tests/common/**` 的最后修正。
- 禁止迁移 R10B/C/D 的完整 targets、修改 production/driver test-support 或复制 compile pipeline。

## Probe matrix

1. type/import/File IR lane：可构造 source、dependency/import，并读取 compiled File IR/type/effect 结果。
2. config/DB/resource lane：可构造 package.yml config、logical DB、static resource，并读取 PackageArtifact 投影。
3. explicit contract lane：只凭 `ServiceContractDefinition` 编译/校验 ServiceContract，不读取 provider source。

## 完成态

1. 三条 representative probe 都 compile/test PASS；缺 API 在本任务统一补 common owner，不能留给 consumer 分支。
2. shared API 仍不返回 service aggregate、legacy unit/runtime holder，不增加场景专用万能 flags。
3. R10B/C/D 只需消费 frozen API，文件 ownership 继续互斥。

## 验证

- representative probe test、targeted rustfmt、legacy helper/type 反向搜索、`git diff --check`；不跑完整 compiler gate。

提交 clean fan-out checkpoint；回报三 lane API/证据和 consumer handoff。
