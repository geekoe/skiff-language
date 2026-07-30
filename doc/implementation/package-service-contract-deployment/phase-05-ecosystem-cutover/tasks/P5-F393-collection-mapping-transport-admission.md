# P5-F393 Collection mapping transport and admission

状态：Ready（F388 C0）。

## 直接父节点

- `P5-F388-legacy-live-service-authoring-audit-result.md`

`package.yml`已接受dependency `collection_name_mapping`，但compiler pipeline随后丢弃，导致mapped
package的collection owner语义不能到达artifact/deployment/runtime。本节点只闭合这条共享事实，不修改
三个legacy service root。

## Worktree

- `/Users/geek/workspace/skiff-p5-f393-collection-mapping-transport`
- branch `codex/p5-f393-collection-mapping-transport`
- base：包含本任务的Skiff phase-05 integration。

## Canonical要求

1. dependency edge上的mapping从authoring input逐跳保留到：
   - compiler source/input requirement；
   - PackageArtifact compile/package requirement；
   - generated deployment binding；
   - RuntimeAssembly/linker/loader validation；
   - Host `ActiveAssemblyContext` activation metadata。
2. runtime解析package collection时使用exact dependency edge mapping，例如
   `package_secret -> mapped_package_secret`；mapping不能只留在diagnostic metadata。
3. mapping是canonical identity fact：
   - 内容或目标变化必须改变拥有该edge的相应artifact/deployment/build identity；
   - 无mapping与empty mapping规范化为唯一表示；
   - map key顺序不影响identity。
4. fail closed：
   - unknown source collection；
   - 两个source collection映到同一target造成collision；
   - 与service自身collection或另一dependency的active target冲突；
   - deployment/assembly mapping与PackageArtifact requirement漂移；
   - reload/activation缺少所需mapping metadata。
5. Skiff尚未发布：若DTO/schema必须变化，直接迁到单一current generation并更新所有直接consumer/golden，
   不dual-read旧shape。

## 写入边界

允许：

- `compiler/input-model/src/dependencies.rs`
- `artifact-model/src/compile_requirements.rs`
- compiler requirement/deployment projection的直接owner
- artifact/deployment/assembly identity与direct DTO（仅确实承载mapping所需）
- linker/loader dependency-edge validation
- `runtime/host/src/loader/active_assembly_context.rs`
- 以上owner直接测试/fixtures/goldens。

禁止：

- F388三个legacy service/package root及其harness；
- test-runner live/package-test evidence；
- Router HTTP/WS、Internals、skiff-packages；
- stable/live/Mongo。

若发现mapping必须进入尚未冻结的外部协议而非artifact/deployment内部事实，返回
`TASK_SCOPE_EXPANDED`；不得用全局collection rename或运行时猜测代替edge mapping。

## 验收

至少覆盖：

- no/empty/single/multi mapping identity规范化；
- mapped、unmapped、unknown、collision及drift正负例；
- fresh PackageArtifact→Deployment→Assembly逐跳receipt；
- isolated Host admission/activation context中exact mapping可见；
- reload后保持；
- 原无mapping dependency fixtures无回归。

运行相关compiler/artifact-identity/deployment/Host聚焦测试、cargo check/fmt和`git diff --check`。用
temporary最小package fixture证明`package_secret`实际投影成`mapped_package_secret`，不操作stable。

写`P5-F393-collection-mapping-transport-admission-result.md`，production/tests/result本地commit，
worktree clean；不merge/rebase/push，不派子Agent。
