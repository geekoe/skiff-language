# P5-F03B：Router Integration Repair

## 输入与owner

- 依赖：P5-R02A PASS的exact F03A seam、P5-R10 PASS的统一endpoint bootstrap、P5-R13 PASS的canonical unary、
  P5-R24 PASS的typed unified WS owner checkpoint及P5-F23E shared generation lifecycle wire。与F03C并行，合流后
  先解锁最终R05，再共同解锁I02。
- branch：`codex/p5-f03b-router-integration-repair`。
- worktree：`/Users/geek/workspace/skiff-p5-f03b-router-repair`。
- 独占`router/**`及直接tests；消费F05 ABI但不回改shared wire/WS authoring规则，不改Rust
  runtime/compiler/test-runner。只提交task branch。

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
pnpm --filter @skiff/router test -- \
  tests/active-assembly-reload.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/host-ingress.test.ts \
  tests/assembly-runtime-endpoint.test.ts
git diff --check
```

聚焦探针覆盖capabilities不掉线、actor/spawn typed response、binary prepare/ACK/commit/register、bootstrap、
serverStream与WS old-generation pin、adapter failure rollback。提交一个commit及证据矩阵，不跑I02/full suite。
