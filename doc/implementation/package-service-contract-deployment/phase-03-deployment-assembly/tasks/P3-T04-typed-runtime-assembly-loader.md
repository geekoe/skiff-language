# P3-T04：Typed RuntimeAssembly Loader / Hydration

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§9、§10、§11、§12、§14。
- 风险/验收组：高风险 typed artifact trust boundary；与 T05、T06合成一次 runtime-link batch只读验收，
  T09覆盖最终链路。
- 当前成熟度：T01 canonical implementation checkpoint；完成后推进 typed loader checkpoint。
- 有效证据状态：本任务 clean commit叠加调度时 exact T01 integration checkpoint。RuntimeAssembly/ref surface、
  loader production call graph、storage resolver、依赖、fixture或测试变化会使证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：T01 checkpoint 已合入 integration；以 T01 canonical fixtures/API 开发，不复制 resolver。
- 解锁：T06。
- branch：`codex/p3-t04-typed-loader`。
- worktree：`/Users/geek/workspace/skiff-p3-t04-loader`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计；若需要 schema amendment，停止并回报 T01 owner。

## 写入范围

独占 `runtime/loader/**` 及其 `Cargo.toml`。不得修改 deployment/artifact、linked-program、linker、host或其它
runtime crate。

## 完成态

1. 新 production loader入口只接受 typed `RuntimeAssembly` 和受信 storage/content resolver；不解析 raw
   `serviceAssembly` index，不以 `ServiceUnit`/`PackageUnit`/display name/source path重建 semantic target。
2. exact refs逐项校验 coordinate、declared content identity、PackageBuildId/local ABI、ServiceProtocol、
   DeploymentArtifactIdentity、AssemblyIdentity；load后再次执行 typed validator。
3. File IR/static resource按 canonical assembly link plan hydrate；缺失、重复、hash/size/path escape/tamper全部在
   linking前失败。
4. 相同 PackageBuildId只load一次并可按 deterministic code slot查询；activation template不在 loader中变成
   mutable runtime owner。
5. canonical empty assembly load成功且返回空 hydrated input。
6. 新入口不带 legacy fallback、dual-read或请求时 lazy load API。旧定义若仍供 Phase 05未迁 consumer编译，必须
   与新 production入口物理隔离且不可被 linker/admission调用。

## 最早风险探针

- tampered assembly/deployment/package/file/resource/ref均给稳定错误并不返回 partial load。
- duplicate refs、same build不同内容、missing link-plan target失败。
- production reverse search证明 typed入口不消费 old `ArtifactIndexPointer` chain。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-loader
rg -n 'ServiceUnit|PackageUnit|serviceAssembly|ArtifactIndexPointer' runtime/loader/src
git diff --check
```

反向搜索需区分 quarantined legacy模块与新 production call graph，并在回报列出证据。只格式化本任务文件；不运行
完整 runtime/phase gate。

## 回报

提交一个 commit，回报 commit、typed loader API、trust boundary、tamper matrix、legacy隔离证据和精确命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
