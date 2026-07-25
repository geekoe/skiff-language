# P5-F288 Open error artifact and contract consumers result

状态：`PASS`；closed throw-set consumers 已删除，artifact/contract identity 已切换到唯一新代际。

## Exact candidate

- implementation commit：
  `cdcd75b7ce787bba749d19c68af7771a07832ca6`
- 直接父任务：
  `P5-F288-open-error-artifact-contract-consumers.md`

## 结果

- callable projection、normalization、validation 和 dependency rebinding 不再生产或读取
  `throw_types`。
- boundary projection、contract normalization/existential/schema closure、deployment value plan 和
  test-runner schema root 不再生产或读取 operation `errors`。
- Rust wire 不输出 `throwTypes` / `errors`；旧字段继续由 strict deserialize 拒绝。
- `throw_origins`、`throws_caller_alias`、`detached_error` 与 F278 same-heap eligibility 均保留。
- WebSocket `maySuspend` 的 deployment 过期负例已与 canonical model 对齐为允许；没有修改 production
  ingress ABI 或 eligibility。

Identity 代际：

| Domain | 新代际 |
| --- | --- |
| File IR prefix | v6 |
| PackageArtifact Local ABI marker / prefix | v2 / v4 |
| PackageArtifact Build marker / prefix | v3 / v5 |
| ServiceProtocol marker / prefix | v4 / v4 |

legacy Package Build/Local ABI、ContractOperation、PackageSchema Type/Index、Operation ABI 与
Publication ABI 算法保持不变。实现中可能抛出的类型变化不进入 Local ABI 或 ServiceProtocol identity；
throw provenance/effects 仍只影响 build。

## 验证

```text
skiff-artifact-identity     89/89
skiff-compiler-contract      2/2
skiff-deployment            52/52
same-heap focused            1/1
cargo fmt --all -- --check  PASS
git diff --check            PASS
```

compiler-compiled、projection-input、projection、input contract dependencies 与 test-runner 的测试枚举，
当时统一被 F286-owned `compiler/core/src/type_closure/mod.rs` 旧 union `variants` consumer 遮挡。
F286 合流后的 combined compiler probe 必须实际执行这些被遮挡测试。

Router 仍硬编码旧 generation，明确交由后续 W2-W transport/router 节点；不得为此加入 legacy fallback。
