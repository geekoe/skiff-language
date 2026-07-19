# P3-R01：Canonical Contract Checkpoint Acceptance

## 角色、输入与证据状态

- 独立只读验收 Agent；不得参与 T01开发、修改文件或创建 commit。
- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§5、§10、§11、§12、§14。
- 调度时由主 Agent提供一个 exact clean integration commit；该 commit必须包含 T01且不包含任何 downstream
  implementation写入。
- 同时读取 `P3-T01-canonical-deployment-assembly-contract.md`、T01自验收矩阵和聚焦测试证据；不预设结论，
  不机械重跑仍有效的完整命令。
- 证据只对该 exact commit及其 dependency/fixture/checker状态有效；artifact/deployment public surface、identity、
  workspace dependency、checker subject或相关测试变化会使验收失效。

## DAG 与风险

- 高风险 schema/identity checkpoint唯一验收 owner。
- R01 PASS才解锁 T02、T03、T04、T05；FAIL返回 T01 owner，不允许 downstream用本地扩字段继续。
- 当前成熟度：T01 implementation checkpoint；PASS不代表 Phase 03稳定候选。

## 必验条款

1. ServiceDeployment/RuntimeAssembly/ref/key/template/link-plan owner单一，strict wire、required semantics与 identity
   newtype分离；不嵌入 legacy aggregate或 storage path。
2. service selector不含 provider build/revision/route；package binding key含 caller build + alias，service key含
   caller build + slot。
3. ServiceContract仍独占 descriptor/schema；deployment/assembly只持 canonical ref + operation ID。
4. identity inclusion/exclusion、canonical order、declared identity tamper、empty assembly均有 mutation证据。
5. checker及 self-test可发现第二 owner、legacy embedding、改名/移动/重复 owner和 test-only伪例外。
6. `skiff-deployment` shell已预建互不争抢 crate root的 projection/assembly module，T02/T03可独立写入。

## 输出

第一行 `PASS` 或 `FAIL`。`FAIL` 列 blocking issue、设计/任务证据、production代码证据、影响、建议 T01 owner
和失效证据；另列 non-blocking follow-up、已运行聚焦命令、未覆盖动态风险。回报必须写明 exact commit。
