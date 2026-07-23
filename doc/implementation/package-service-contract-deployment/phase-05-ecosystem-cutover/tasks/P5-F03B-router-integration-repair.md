# P5-F03B：Router Integration Repair

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、6、8、9、10条，§5、§6.2、§7、§12及§14。
Router只能从active RuntimeAssembly snapshot与typed deployment/ingress事实选择target；service call必须经dispatcher切换
ActivationContext，长连接必须固定其建立时generation并在可观测drain后释放。

## 输入与owner

- 依赖：P5-R02A PASS的exact F03A seam、P5-R10 PASS的统一endpoint bootstrap、P5-R13 PASS的canonical unary、
  P5-R24 PASS的typed unified WS owner checkpoint及P5-F23E shared generation lifecycle wire。与F03C并行，合流后
  先解锁最终R05，再共同解锁I02。
- DAG节点F03B；风险高，验收分组为Router production consumer。进入状态为F23E exact integration checkpoint，派发时给
  commit/tree/Cargo.lock。完成后仍只是Implementation Checkpoint，不能单独解除R05/I02。
- branch：`codex/p5-f03b-router-integration-repair`。
- worktree：`/Users/geek/workspace/skiff-p5-f03b-router-repair`。
- 独占`router/**`及直接tests；消费F05 ABI但不回改shared wire/WS authoring规则，不改Rust
  runtime/compiler/test-runner。F23E的protocol schema/codec/golden为冻结输入，只允许consumer import，不得改其字段或
  接受集合。只提交task branch。

## 完成态

1. 消费F09/R10已通过的唯一Runtime WebSocket endpoint/dispatcher与binary activation bootstrap，不回建
   `AssemblyRuntimeEndpoint`或第二capability/session owner；本任务只完成其余store、gateway、participant与pin职责。
2. 生产activation store/snapshot loader通过F03A compiler adapter消费T01 store。删除
   `FileAssemblyActivationStateStore`的Node path/CAS/reducer与manual RuntimeAssembly path/identity decode；
   memory fake只留直接tests。Router startup先idempotent ensure empty generation-0 bootstrap，再按exact state
   建snapshot；只有匹配committed tuple的healthy registration可接流量。
3. participant集合消费F09已完成capabilities握手的healthy runtime连接；initial empty registration与后续旧
   generation registration都可参加prepare，commit前仍检查连接/ACK exact tuple。全部control走binary frame。消费F23E
   release/ack，在client/policy/gateway close与runtime disconnect上幂等释放完整connection pin。
4. HTTP gateway按snapshot中exact ServiceContract operation选择unary/serverStream，发送canonical nested
   assembly routing；不伪造build/target/service selector。WS connect建立generation-pinned connection，receive
   继续发送原assembly/generation/operation/ingress；cutover后旧连接只drain，新连接选新generation。
5. rewrite/header/query selector仍fail closed；health同时暴露capability connection与committed registration，
   不把连接等同于已admit registration。

`extra-review`约束：统一endpoint和store client是职责边界，不把逻辑重新堆进server/runtimeProtocol；新文件
超过500行必须有单一明确职责且无重复dispatcher/validator。

D11把本任务原完成态1及完成态3的bootstrap/session前半段提前拆为F09/R10，以解除F04真实Host gate的DAG环；
R10不解锁本任务，R05 PASS后仍由F03B唯一完成store/snapshot/gateway/participant/pin剩余职责。

D14把normal HTTP unary canonical writer/dispatch提前拆为F12/R13以解除F04环；本任务消费该已通过lane，不回建flat
header、build rewrite或第二writer，R05后仍完成serverStream/WS gateway、snapshot/pin/drain等剩余职责。

## 验证

```bash
pnpm --filter @skiff/router type-check
pnpm --dir router exec vitest run \
  tests/active-assembly-reload.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/host-ingress.test.ts \
  tests/assembly-runtime-endpoint.test.ts
git diff --check
```

新增consumer direct tests必须通过同一direct Vitest命令显式列出，测试数非零。聚焦探针覆盖capabilities不掉线、
actor/spawn typed response、binary prepare/ACK/commit/register、bootstrap、serverStream与WS old-generation pin、
F23E acquire/release/ack/reject、runtime-session disconnect cleanup及adapter failure rollback。提交一个clean commit及证据
矩阵；禁止I02、R05 real transcript、full/I16/Host/stable，不merge/push。Router store/gateway/endpoint/F23E wire或
Runtime activation schema变化会使证据失效。
