# P5-F03B：Router Integration Repair

## 输入与owner

- 依赖：P5-R02A PASS的exact F03A seam及P5-R05 PASS的typed unified WS checkpoint。与F03C并行，合流后共同解锁I02。
- branch：`codex/p5-f03b-router-integration-repair`。
- worktree：`/Users/geek/workspace/skiff-p5-f03b-router-repair`。
- 独占`router/**`及直接tests；消费F05 ABI但不回改shared wire/WS authoring规则，不改Rust
  runtime/compiler/test-runner。只提交task branch。

## 完成态

1. 生产只保留一个Runtime WebSocket endpoint/dispatcher。把assembly.activation接入既有
   `RuntimeEndpoint`的binary分发，完整保留runtime.capabilities、health、actor/spawn、response、cancel、
   connection.send；删除或降为无production owner的`AssemblyRuntimeEndpoint`，不得复制缩减协议。
2. 生产activation store/snapshot loader通过F03A compiler adapter消费T01 store。删除
   `FileAssemblyActivationStateStore`的Node path/CAS/reducer与manual RuntimeAssembly path/identity decode；
   memory fake只留直接tests。Router startup先idempotent ensure empty generation-0 bootstrap，再按exact state
   建snapshot；只有匹配committed tuple的healthy registration可接流量。
3. participant集合来自已完成capabilities握手的healthy runtime连接；initial empty registration与后续旧
   generation registration都可参加prepare，commit前仍检查连接/ACK exact tuple。全部control走binary frame。
4. HTTP gateway按snapshot中exact ServiceContract operation选择unary/serverStream，发送canonical nested
   assembly routing；不伪造build/target/service selector。WS connect建立generation-pinned connection，receive
   继续发送原assembly/generation/operation/ingress；cutover后旧连接只drain，新连接选新generation。
5. rewrite/header/query selector仍fail closed；health同时暴露capability connection与committed registration，
   不把连接等同于已admit registration。

`extra-review`约束：统一endpoint和store client是职责边界，不把逻辑重新堆进server/runtimeProtocol；新文件
超过500行必须有单一明确职责且无重复dispatcher/validator。

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
