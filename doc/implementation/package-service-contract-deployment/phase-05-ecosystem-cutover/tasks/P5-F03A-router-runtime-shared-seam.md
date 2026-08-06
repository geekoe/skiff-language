# P5-F03A：Router / Runtime Shared Seam Repair

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 输入、分类与DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md` §2、§5–§6、§10、§12–§14；
  执行输入为P5-R02预审在`b47ddf7` / tree `18292f1a`确认的七个T03/T04 blocker。
- 本任务是repair W0：先串行冻结binary activation framing、canonical assembly request header与跨进程
  canonical store seam；不实现Router endpoint或Runtime host consumer。
- 依赖：T02/T03/T04已合流。解锁：P5-R02A独立只读验收；PASS后F03B/F03C并行。
- branch：`codex/p5-f03a-router-runtime-shared-seam`。
- worktree：`/Users/geek/workspace/skiff-p5-f03a-router-runtime-seam`。
- 使用新的开发Agent；五分钟内产生真实code edit。只提交task branch，不merge、不push。

## 写入范围

- `router/src/protocol/**`、`runtime/transport/**`中共享frame/header codec及直接tests；
- `compiler/driver/**`与`compiler/driver/bin/skiff-compiler.rs`中的strict internal ecosystem-store adapter；
- `deployment/src/storage/io.rs`及其直接tests中一处canonical store并发目录创建修复：仅容忍
  `create_dir`竞态返回的`AlreadyExists`，随后必须重新检查目标是真实目录、不是symlink，并继续执行
  root containment校验；不得把path、identity、lock或CAS规则复制到adapter；
- `cross-system-fixtures/package-service-ecosystem/**`的TS/Rust golden/mutation corpus；
- 必要public导出。除非已有manifest不能编译，不改root Cargo/lock。

禁止修改router server/endpoint/gateway/coordinator、runtime host/driver/admission、test-runner或旧路径删除。

## 冻结接口

1. assembly control只使用现有typed binary runtime-frame codec。新增header：

   ```text
   { schemaVersion, type: "assembly.activation", control: <T01 AssemblyActivationControl> }
   ```

   payload必须为空。Router与Runtime两端都不得发送/接受text control、裸binary JSON或另一个control envelope。
   direction仍严格：router只发prepare/commit/abort；runtime只发prepared/reject/register。
2. canonical gateway `request.start`不再伪造build/service字段，使用互斥的nested routing variant：

   ```text
   routing: {
     kind: "runtimeAssembly",
     assemblyIdentity,
     assemblyGeneration,
     contractOperationId,
     ingress: { protocol, host, method, path }
   }
   ```

   `assemblyGeneration`使用activation safe generation值域；method显式`string|null`。assembly routing与legacy
   `target/buildId/serviceProtocolIdentity/service/version/selector`不得同现。TS validator与Rust
   `deny_unknown_fields` decoder消费同一fixture；本任务不删除legacy variant。
3. `skiff-compiler`提供不出现在public四对象help中的strict internal ecosystem-store JSON adapter，所有操作
   直接委托`CanonicalArtifactStore`，不得在Node复制path/identity/CAS：
   - `ensureEnvironmentBootstrap`：仅在state完全不存在时，发布canonical空RuntimeAssembly并原子初始化
     generation 0；已存在则bit-identical返回，partial/tampered不得覆盖。空assembly只是四对象中的
     RuntimeAssembly，不是第五对象；未有healthy exact registration前Router仍不接业务流量。
   - `readEnvironment`、`prepareEnvironment`、`abortEnvironment`、`commitEnvironment`：字段与T01 state/CAS一致。
   - `readRouterSnapshot`：从exact RuntimeAssembly ref经typed store加载、重算identity并返回assembly及其
     ServiceContract records，供Router导出ingress/mode；不返回raw path或latest pointer。
   请求/响应均`deny_unknown_fields`、stdin/stdout单JSON、stderr稳定错误，不引入artifact kind/common envelope。
4. golden corpus覆盖binary frame bytes、全部control variant/direction、payload/text/unknown mutation、canonical
   request routing与legacy-field collision，以及bootstrap/read/CAS/snapshot adapter正负例。

并发bootstrap测试在实现中真实暴露`CanonicalArtifactStore::prepare_destination`的TOCTOU：两个调用者同时
观察到父目录缺失后，后创建者会收到`AlreadyExists`。该底层最小修复属于本任务显式授权范围；验收必须
证明并发调用收敛到同一generation-0 state，同时symlink/non-directory与越界路径仍fail closed。

## 完成态与验证

- TS/Rust对同一binary frame和request header bit-level/typed结果一致；mutation双端fail closed。
- Router/Runtime后续只需消费接口，不再协商字段/framing；Router可通过单一Rust adapter消费T01 store。
- shared文件若已过大，按codec/fixture职责拆模块；不得继续扩大约3955行`runtimeProtocol.ts`中的重复规则。

```bash
cargo test -p skiff-runtime-transport assembly_activation_frame
cargo test -p skiff-runtime-transport runtime_assembly_request_start
cargo test -p skiff-compiler ecosystem_store
pnpm --filter @skiff/router type-check
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
git diff --check
```

不跑endpoint/host/I02/full gate。提交一个commit，回报frame/header/store表、mutation、反搜、exact commit/tree
与clean状态。
