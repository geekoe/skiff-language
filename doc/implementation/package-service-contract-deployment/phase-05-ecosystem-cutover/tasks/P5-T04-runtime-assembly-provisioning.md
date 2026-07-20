# P5-T04：Runtime Assembly Provisioning / Replica Registration

## 权威输入与DAG

- 设计：`doc/architecture/package-service-contract-deployment.md` §2、§6–§12、§14–§15。
- 依赖：R01 PASS exact checkpoint；与T02/T03/T05同级，解锁R02。
- 风险：高；production load/link/admit、atomic generation、runtime lifecycle。
- branch：`codex/p5-t04-runtime-assembly-provisioning`。worktree：`/Users/geek/workspace/skiff-p5-t04-runtime`。
- 当前共享状态是R01 PASS的implementation checkpoint；完成后仍只是R02 batch candidate。使用新的开发
  Agent；证据对T01 resolver/wire、owned runtime production/tests、Cargo或fixture变化失效。
- 五分钟内编辑production文件；不用test-only resolver或旧graph adapter越过T01缺口。

## 写入范围

独占 `runtime/transport/**`、`runtime/loader/**`、`runtime/host/**` 中control/reload/resolver/admission/
generation/registration/lifecycle及直接tests。不改`runtime/package-test`、test-runner、router、scripts、
artifact schema或verify接线。

## 完成态

1. runtime startup与router.control都将exact assembly ref交给production resolver，经typed loader/linker/
   `admit_runtime_assembly`构建candidate active context；不用`services: Vec::new()`或old service config。
2. 只load/link/admit全部成功才原子替换active generation并注册assembly。任一失败保留
   旧context/registration，且没有partial activation/callback/state table泄漏。
3. replica id与AssemblyIdentity分离；多个runtime-home可加载同一assembly，各自package code
   在replica内只链接一次，activation mutable owner不跨replica/不跨deployment共享。
4. request从active generation canonical ingress进入Phase 04 single dispatcher；整个request/stream pin住
   generation，期间artifact I/O为零。
5. 删除production old pointer graph/lazy service load/per-service queue_registers/artifactRoots/serviceConfig
   consumer；不给legacy loader增加fallback。
6. runtime disconnect/reconnect只重注册已admit exact assembly；未完admission不发送空/伪per-service
   registration。

## 最早探针与唯一验证 owner

- startup加载canonical fixture后registration>0；tampered dependency/admission mismatch保留旧generation。
- 两replica exact identity/independent owner正例；一个replica退出不影响另一个。
- request resolver/file-open spy为零；stream结束后旧generation才可drain。

```bash
cargo test -p skiff-runtime-loader --test runtime_assembly_content_resolver
cargo test -p skiff-runtime-host --test active_runtime_assembly
cargo test -p skiff-runtime-transport --test assembly_replica_registration
node scripts/check-runtime-crate-dag.mjs
git diff --check
```

不跑完整runtime/live/chat。提交一个commit并合入Skiff integration branch，回报startup/control/reload/register链、旧路径反向
搜索、资源清理证据及自验收矩阵。
