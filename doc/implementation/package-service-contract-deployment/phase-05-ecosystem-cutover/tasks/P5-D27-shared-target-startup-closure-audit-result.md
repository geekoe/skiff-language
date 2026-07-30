# P5-D27：Shared-target Startup Closure Audit Result

`D27 AUDIT COMPLETE`

D27A/B由互不重叠的全新只读Agent审计G16C诊断因果性与shared-target/B-root启动差异；未修改代码，未运行
full/Host/stable，也不分别给阶段verdict。没有证据证明新的production runtime或artifact-identity根因，没有公共契约、
架构职责或业务语义决策。

## D27A：Gate因果证据

v5 selector固定先遍历全部stdout、再遍历全部stderr，然后取第一个`kind !== diagnostic`的候选，没有severity排序。
production控制流先向stderr写supervisor failure，随后在shutdown向stdout写generic startup failure；两个独立pipe没有
可恢复的跨流时序，ledger也未保留多条候选。G16C因此以generic stdout覆盖因果stderr。

当时source-suite的startup phase marker只会在isolated runtime ready之后出现，故pre-readiness失败只能记录
phase/subject unknown。Router/Runtime因果日志写入owned inner instance，cleanup后被删除，v5 ledger未保留其有界副本。
确定缺口是Gate selector、pre-readiness phase marker与证据保留，不是设计语义；对应修复为F21A的确定性causal rank及最多
3条bounded diagnostics，以及F21B在调用runtime owner前发出的startup marker。

## D27B：B-root/shared-target路径

P26S与G16C候选使用相同startup production代码；P26S通过时使用integration root与owned cold target，G16C失败时使用
B worktree与由A-origin预热的shared target。I16只覆盖compiler/test-runner/smoke artifact与root materialization，未启动
supervisor，也未验证runtime/artifact-identity启动，因此不能把未审计范围写成已证实production根因。

闭合动作冻结为P27S非full复现：owned A/B detached worktrees与空shared target；A build后B build并检查四crate Fresh及
artifact差异；从owned task cwd调用B-root isolated runtime的empty callback；在cleanup前保留bootstrap、supervisor、
readiness、Router/Runtime日志与cleanup证据。若ready则再决定是否需要下游std-only；若pre-readiness失败则按真实component
证据创建新owner修复。P27S只负责诊断事实，不给阶段verdict。

```text
G16C pre-readiness FAIL
├─ F21A：Gate causal diagnostic collection
├─ F21B：source-suite startup phase marker
└─ P27S：shared-target + B-root exact non-full reproduction
       └─ F21C：下一独立DAG节点（pending，须先建立任务合同）
```
