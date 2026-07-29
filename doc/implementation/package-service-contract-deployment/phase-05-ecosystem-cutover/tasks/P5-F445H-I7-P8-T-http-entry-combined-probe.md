# P5-F445H I7 P8 T HTTP entry combined probe

状态：

```text
BLOCKED_BY = K,H,R
PRODUCTION_WRITE = NO
```

## 1. Parent and candidate gate

- 直接父节点：
  `P5-F445H-I7-P8-D0-http-entry-test-authority-result.md`
- ancestry floor：
  `3a87d37f81a04c249f308b311bd91dcfdf3a8aa3`
  （tree `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`）
- dispatch前必须把K/H/R已集成后的精确Skiff commit/tree写入本任务result；未冻结时不可创建WT。
- DAG：`K + H + R -> T -> I`
- integration owner：`/root/phase05_integration_steward`

## 2. Scope

T只增加一个hermetic `kind: test` fixture和跨层probe，不修改production：

- 显式`http.yml`引用`*.test.skiff` wrapper；
- unary self-ingress返回完整body；
- raw HTTP stream经`std.http.stream`消费；
- entry内部outbound HTTP/package effect命中父inline double；
- stream consumer break到Router/Runtime取消链；
- 第二个active self-ingress被拒绝，前一个结束后顺序调用成功；
- selector/Host/body framing/hop-by-hop header覆盖在发送前失败；
- 非self origin仍命中普通double。

建议写集：

```text
test-runner/fixtures/http-entry-test-service/**
test-runner/tests/http_entry_test_service.rs
```

不得修改K/H/R生产文件；发现缺口时按owner返回，不在probe中加shim。

## 3. RED / GREEN

在`3a87d37` ancestry floor上应用同一fixture，证明`emitResponseStream outside ... context`或其直接
等价RED；随后在K/H/R合流候选运行同一真实入口到GREEN。证据至少包括：

```text
cargo test --locked -p skiff-test-runner --test http_entry_test_service -- --nocapture
git diff --check
```

probe必须经过真实isolated Router business port；直接调用handler、伪造response sink或mock Router不算
GREEN。完成后记录精确candidate、进程/端口清理和未运行的昂贵gate。
