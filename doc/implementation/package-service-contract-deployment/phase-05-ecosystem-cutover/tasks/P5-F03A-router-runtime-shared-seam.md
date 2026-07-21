# P5-F03A：Router / Runtime Shared Seam Repair

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
