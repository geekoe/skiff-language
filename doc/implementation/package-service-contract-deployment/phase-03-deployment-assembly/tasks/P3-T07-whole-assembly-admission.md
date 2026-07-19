# P3-T07：Whole-assembly Host Admission / Atomic Swap

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2.6–§2.10、§10、§11、§12、§14。
- 风险/验收组：高风险 atomic admission/concurrency；与 T08合流后做 runtime-admission batch验收。
- 当前成熟度：T06 pre-admission candidate；完成后推进 whole-assembly admission checkpoint，不是稳定候选。
- 有效证据状态：本任务 clean commit叠加调度时 exact T06 integration checkpoint。candidate API、host active/
  candidate state、request entry、依赖、fixture、concurrency测试或环境变化会使相关证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：T06 已合入 integration。
- 解锁：T09。
- branch：`codex/p3-t07-assembly-admission`。
- worktree：`/Users/geek/workspace/skiff-p3-t07-admission`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计；不得以保留 request-time lazy load来绕过
  candidate/admission缺口。

## 写入范围

独占 `runtime/host/src/loader/**`、whole-assembly candidate/active state、admission/health owner，以及
`runtime/host/src/host/request_entry.rs` 中删除 request-time load所需的最小改动和 `runtime/host/Cargo.toml`。
不得实现 Phase 04 dispatcher/materialization，也不得修改上游 crate。

## 完成态

1. host以完整 assembly启动 candidate build：typed load → link → validate → admit；只在全部成功后一次性替换
   active assembly。
2. 失败保留旧 active assembly和既有请求可用性；candidate与 active状态不会混合，concurrent reload具备明确
   generation/serialization规则。
3. health/control-plane internal state可观察 active AssemblyIdentity、candidate identity/stage、最后 admission
   success/error和时间；错误不泄漏 secret material。
4. canonical empty assembly可成为 active，业务 service/ingress lookup fail closed。
5. request entry只查 active assembly route/activation template；没有 artifact I/O、pointer parse、load/link或
   `lazy_load_request_service`。
6. drain/reload边界保留 whole-assembly一致性；本阶段不定义 router wire、release pointer或多 assembly routing。
7. production admission不调用 legacy service-level loader/linker fallback，不把旧 `ServiceUnit`/`PackageUnit`
   转换成新 candidate。

## 最早风险探针

- active A后 candidate B在 load/link/admission各阶段失败，active identity/route均保持 A。
- concurrent B/C reload不能出现 B code + C templates；成功替换只有完整 generation。
- request期间反向 instrumentation证明零 artifact I/O；empty active所有 lookup稳定失败。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-host assembly_admission
cargo test -p skiff-runtime-host atomic_reload
cargo test -p skiff-runtime-host request_entry
rg -n 'lazy_load_request_service|ArtifactIndexPointer|serviceAssembly' runtime/host/src/host/request_entry.rs runtime/host/src/loader
git diff --check
```

若测试名不同，回报精确 filter。只格式化本任务文件；不运行完整 runtime/phase gate或本地 stable instance。

## 回报

提交一个 commit，回报 commit、admission状态机、atomicity/concurrency证据、request零I/O证据、health字段与命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
