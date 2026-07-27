# P5-F432A1 Test-runner HTTP surface exhaustive closure

状态：Ready。F432A后的单文件机械闭合。

## 直接父节点

- `P5-F432A-test-runner-optional-handler-checkpoint-result.md`

父result记录已集成safe checkpoint、唯一E0005 owner和全test-runner反搜结论，并继续追溯F432 wave
与权威设计。

## 输入与DAG

| commit | tree |
| --- | --- |
| `d1b2951d0abd892fc6d6698c2bf964cb9caed214` | `889b9fd0d4915f018bcbab7a94d51ba79dbaefe0` |

本节点完成F432A共享compile checkpoint；通过后才解除fixture/tooling、AIHub与Agine后继。

## 唯一写入范围与实现

只允许：

```text
test-runner/tests/package_service_contract_deployment.rs
```

以及本leaf result。

把`package_test_http_fixture_is_zero_operation_reference_closed_and_fail_closed`中的plain
`let GatewayProtocolSurface::Http(surface)`改为显式`let ... else`或exact match：

- HTTP保持正例；
-任何非HTTP surface都立即以清楚fixture invariant失败；
- 不允许忽略`WebSocketConnect`、用unreachable unchecked或改变production schema。

禁止修改其它test-runner、artifact/compiler/runtime/Router、fixture/scripts、Internals或
skiff-packages。若再发现新production/test owner，返回精确blocker，不扩大范围。

## 验证

本Agent恢复并唯一完成F432A剩余验证：

```bash
cargo check -p skiff-test-runner
cargo test -p skiff-test-runner --lib
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  package_test_http_fixture_is_zero_operation_reference_closed_and_fail_closed
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  i02_submit_probe_is_private_http_gateway_not_service_operation
cargo fmt --all -- --check
git diff --check
```

两个filter必须各实际发现并通过1项；不得把0 tests当PASS。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f432a1-http-surface`
- 分支：`codex/p5-f432a1-http-surface`

启动后5分钟内实际修改。提交implementation，再新增并提交
`P5-F432A1-test-runner-http-surface-exhaustive-closure-result.md`。返回commit/tree、discovery与
clean状态。不得merge、rebase、push、stable/live或承接后继。
