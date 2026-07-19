# P3-R03：Runtime Load / Image / Link Checkpoint Acceptance

## 角色、输入与证据状态

- 独立只读验收 Agent；不得参与 T04/T05/T06开发、修改文件或创建 commit。
- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§3、§6、§9、§10、§11、
  §12、§14。
- 主 Agent提供 exact clean integration commit；必须包含 R02已验收状态和 T04/T05/T06，不包含 T07/T08A/T08B
  写入。
- 同时读取三个任务合同、自验收矩阵和聚焦证据；只做必要抽查，不重复完整 runtime/phase gate。
- 证据只对 exact commit的 typed loader、linked image、linker API/call graph、dependencies、fixtures/tests有效；
  这些表面变化会使验收失效。

## DAG 与风险

- 高风险 typed trust boundary、shared code/activation ownership和 linker checkpoint的唯一批次验收 owner。
- R03 PASS才解锁 T07、T08A、T08B；FAIL按 loader/image/linker边界退回 T04/T05/T06 owner。
- 当前成熟度：pre-admission implementation candidate；PASS仍不是 whole-assembly稳定候选。

## 必验条款

1. loader只接收 typed RuntimeAssembly/exact refs，tampered deployment/contract/package/File IR/resource/link plan在
   partial output前失败；empty assembly可 hydrate。
2. loader hydrate immutable canonical ServiceContract store，按 ref+operation可取 descriptor/value plan；不复制
   descriptor owner，不丢失 Phase 04 handoff。
3. linked image按 PackageBuildId只持一份 immutable code/resource/callable view，不拥有 activation mutable state。
4. package direct call走 alias→expected ABI→exact build→callableLinks；service call保留 caller build+slot+operation+
   protocol，不 patch provider executable。
5. linker candidate保留 shared image、activation templates和 contract store；wrong ABI/callable/protocol/template/
   ref均 fail closed，无 partial candidate。
6. production call graph不读取 legacy DTO、raw JSON/source/display target，不提供 fallback/dual-read/lazy load。

## 输出

第一行 `PASS` 或 `FAIL`。FAIL按 T04/T05/T06 owner列 blocking issue、设计/任务/production证据、影响与失效
证据；另列 non-blocking、聚焦命令、动态风险和 exact commit。
