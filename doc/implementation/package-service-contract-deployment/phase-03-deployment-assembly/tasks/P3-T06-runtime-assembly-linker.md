# P3-T06：RuntimeAssembly Linker Checkpoint

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§3、§6、§9、§10、§12、§14。
- 风险/验收组：高风险 runtime linking boundary；与 T04、T05合成一次 runtime-link batch只读验收。
- 当前成熟度：deployment/assembly/loader/image implementation checkpoints；完成后推进 pre-admission candidate。
- 有效证据状态：本任务 clean commit叠加调度时 exact T03–T05 integration checkpoint。任何上游 public API、
  link plan、linker production call graph、依赖、fixture或测试变化都会使证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：T03、T04、T05 已合入 integration；消费其真实 public API，不复制 loader/resolver/image owner。
- 解锁：T07、T08。
- branch：`codex/p3-t06-assembly-linker`。
- worktree：`/Users/geek/workspace/skiff-p3-t06-linker`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计；上游接口不闭合时回报具体 producer contract，
  不用 legacy adapter拼接。

## 写入范围

独占 `runtime/linker/**` 及其 `Cargo.toml`。不得修改 loader、linked-program、deployment/artifact、host或其它
runtime crate。

## 完成态

实现唯一 production checkpoint：

```text
validated RuntimeAssembly + fully hydrated assembly input
  -> link exact package code once
  -> validate activation-relative templates
  -> AssemblyLinkedCandidate
```

1. linker只消费 typed assembly/loader/image API；不读 raw JSON、`ServiceUnit`、`PackageUnit`、raw
   `serviceAssembly`、display/source path或 File IR opaque signature来猜 semantic target。
2. package direct call按 canonical exact chain链接，并验证 requirement alias、version、local ABI、build、callable
   link；service call只验证 caller-relative binding slot、operation与 protocol，保留运行时 activation-relative thunk。
3. 所有 contract operation binding、ingress、service/config/state/resource template与 code slot互相完整且无
   dangling/duplicate；activation A/B共享 build只共享 image，不共享 mutable owner。
4. candidate包含 immutable shared image和 activation templates，不实例化 Phase 04 `ActivationContext`，不执行
   materialization/dispatcher/callback/cancel。
5. tampered ref/File IR/resource/link plan/template在 candidate返回前失败；不返回 partial image。
6. empty assembly链接成功，candidate无 route/activation/code；lookup均 fail closed。
7. 旧 service-specific `LinkedProgramImageBuild` chain不能成为新 production返回路径；不提供 fallback/dual path。

## 最早风险探针

- 两个 activation复用同 build但绑定同一 service slot到不同 provider，thunk结果必须按 activation区分。
- package slot/service slot均使用 0且 caller build不同，不发生全局 key碰撞。
- wrong ABI、missing callable、provider protocol mismatch、template tamper、ingress collision全部在 link/admit前失败。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-linker
rg -n 'ServiceUnit|PackageUnit|serviceAssembly|serde_json::Value' runtime/linker/src
git diff --check
```

只格式化本任务文件；不得运行完整 runtime/phase gate。

## 回报

提交一个 commit，回报 commit、candidate API、package/service linking证据、tamper matrix、legacy反向搜索及命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
