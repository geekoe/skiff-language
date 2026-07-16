# Phase 02：Config-only ServiceUnit Cutover

状态：`outline-only`。Phase 01 验收并合并 `main` 前不得据此派发实现 Agent。

## 目标

把 source-bearing service publication直接切换为无源码的 deployment projection：用户代码只来自
PackageUnit graph，ServiceUnit引用 root package并拥有部署/activation metadata。阶段结束时仓库
中不再存在 service source compile production path。

## 进入条件

- Phase 01独立验收 PASS并已合并/清理 worktree。
- PackageUnit已包含 typed service requirements、code/effect/link和boundary contract。
- 下一轮路径审计完成，所有要实质修改的重复owner/超长文件已升级为前置任务。

## 预计工作域

1. 定义 config-only service manifest：service id/version、root package ref、具名operation surface
   mapping、
   dependency binding、ingress、config/state、timeout和activation policy。
2. compiler pipeline先编译/读取 PackageUnit closure，再从 code contract投影 ServiceUnit；deployment
   projection不读取 AST，不复制用户 executable body。
3. 收窄 ServiceUnit schema：删除 files/source/resources ownership，保留 root package、operation/
   protocol、ingress、state/config owner和deployment metadata。
4. `ArtifactGraph`、loader、linker、`LinkedProgramImage`改成有序 package graph +显式 root package
   slot；删除 `UnitAddr::Service`、`service_files`和隐式 service code root。
5. activation/eval从 root PackageUnit定位 executable，但 service identity/config/DB/recoverable owner
   仍来自 deployment activation。
6. compiler CLI、dev watch、release/registry publication和package-test装配同步切换。
7. 删除 `PublicationInput::Service`、`PublicationCompilePolicy::Service`、service source root和对应旧
   fixture/测试；不保留兼容 reader。

阶段内 service-to-service call可以暂时使用现有transport作为迁移桥，但它必须消费 Phase 01 的
Boundary ABI，且不能重新获得service-owned code。现有transport不能表达的callback/native/
stream plan在deployment时fail closed，不为短命桥接扩建第二套projection。该桥只允许存在到
Phase 03切换production dispatch；Phase 04负责物理删除旧transport代码。

## 预计验收

- 一个没有任何 `.skiff` source的service config可部署指定PackageUnit operation。
- 同一PackageUnit可被两个service deployment使用；code identity相同，service id/config/state
  owner不同。
- ServiceUnit/LinkedProgramImage JSON和Rust model中不再有service-owned File IR。
- loader/eval通过显式root package执行入口，package dependency ordering deterministic。
- repo内无service source compiler production entrypoint、dual manifest reader或legacy schema。
- 现有service行为fixture完成语义迁移，而不是只做字段替换。

## 细化前必须裁决

以下不是当前阻塞；Phase 02细化时必须由Phase 01产物和canonical文档唯一确定，否则询问用户：

- root package使用registry identity、workspace source input还是两者分层表达；
- DB/spawn/actor/recoverable declaration属于code contract还是deployment选择，以及state owner如何编码；
- 具名operation surface mapping的最终YAML拼写和诊断位置；
- Phase 02临时transport桥最小保留边界。

## 可调整点

若 loader/image cutover与 compiler publication cutover无法形成一个可运行阶段，可在细化时拆成
02A/02B，但02B验收前不得把中间dual production path合入 `main`。
