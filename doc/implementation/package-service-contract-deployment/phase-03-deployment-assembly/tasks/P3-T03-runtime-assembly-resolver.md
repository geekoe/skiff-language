# P3-T03：RuntimeAssembly Resolver / Binding Templates

> 已完成的历史任务；2026-07-30后不得重发。RuntimeAssembly不再生成config/state/resource template；
> activation另行接收RuntimeConfigSnapshotRef，service DB由平台派生。

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§5、§6.2、§10、§11、§12、§14。
- 风险/验收组：高风险 assembly closure/provider resolution；R02在同一 checkpoint分别判定
  deployment/assembly边界。
- 当前成熟度：R01已验收 canonical implementation checkpoint；完成后推进 assembly resolver checkpoint，
  不是稳定候选。
- 有效证据状态：本任务 clean commit叠加调度时 exact R01 integration checkpoint。deployment/assembly surface、
  selector规则、依赖、fixture或测试变化会使证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：R01 PASS；只消费 T01 frozen `ServiceDeployment` DTO/validator/fixture，不调用 T02 projection函数。
- 解锁：R02 assembly verdict；R02 PASS 后与 T04/T05共同解锁 T06。
- branch：`codex/p3-t03-assembly-resolver`。
- worktree：`/Users/geek/workspace/skiff-p3-t03-assembly`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计；T01 surface不足时回报 owner，不复制 projection
  或 identity规则。

## 写入范围

独占 `deployment/src/assembly/**` 及 module export/tests。不得修改 projection、artifact schema/identity、compiler
或 `runtime/**`。

## 输入与完成态

实现唯一 pipeline：

```text
explicit root refs + candidate ServiceDeployments + ServiceContracts + PackageArtifacts
  -> resolve closure -> RuntimeAssembly
```

1. 按 root set闭合 service cycle与package dependency graph；A→B→A必须成功且不递归爆栈。
2. 每个 service requirement在 candidate set中按
   `serviceId + contractVersion + expectedProtocolIdentity` 恰好找到一个 deployment；零/多 provider、协议不符、
   remote-only closure失败，不选择“最新”或按 revision/display name猜测。
3. 每个 package edge按 `(callerBuildId, alias)` 校验 packageId、exactVersion、expectedLocalAbi并选择 exact build；
   同 caller edge冲突失败，不允许 activation-relative package build。
4. 相同`PackageBuildId`只生成一个deterministic code/link slot；每个deployment activation仍生成独立
   service binding template，template key包含activation与caller build。
5. service binding template保存 activation-relative provider target与 contract operation，不把 call site全局 patch到
   provider executable；第一版 binding kind只有 `InProcessBoundary`。
6. global ingress selector必须唯一；collision全局失败，不 last-wins。
7. 输出 deterministic roots/closure/link plan/templates，assign并 validate AssemblyIdentity。
8. 空 root set生成合法空 assembly；任何 lookup目标不存在但 projection本身成功。

## 最早风险探针

- service cycle、package diamond、两个 activation共享 build、不同 package都使用 slot 0。
- service零/多 provider、package version/local ABI/build mismatch、template dangling/duplicate、ingress collision。
- map insertion/root input顺序不改变 identity；resolved provider/build/template变化会改变 identity。

## 唯一验证 ownership

```bash
cargo test -p skiff-deployment assembly
git diff --check
```

只格式化本任务 Rust 文件；不得运行完整 phase gate。

## 回报

提交一个 commit，回报 commit、closure/selector算法摘要、正负例矩阵、empty assembly证据及精确命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
