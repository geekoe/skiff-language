# P5-F360 typedJson unary correction result

状态：Completed（C2 compiler语义纠正；只修正F357误接受的
`typedJson + Stream<T>`，shared wire与runtime/router consumer不在本leaf范围）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `344dae7fe29711709d3c435d6cc3e69726451456` | `07194037e113dc92c319a53b4ebb19cd33c62185` |
| task checkpoint | `81e2e03cbdfa00792a1298c7000adcac3396af6b` | `e731a26c8aae15e8168f140c6d0e32d7f2590526` |
| production/tests | `d07d1ae85c984fecec778b6d20680d0a538a9ded` | `4479f5df0a3931a87b85e2862b8dd489ce1cc561` |

工作分支为`codex/p5-f360-typed-json-unary`，worktree为
`/Users/geek/workspace/skiff-p5-f360-typed-json-unary`。本leaf没有merge/rebase integration，
没有运行workspace/root、stable/live，没有修改shared DTO、identity preimage、runtime wire、
lockfile或三仓库service源码，也没有push。

## 2. Compiler correction

- `http_gateway_projection::project_handler_return`识别最外层exact `Stream<T>`后，先按adapter
  kind分流。`typedJson`立即返回entry-local `handler` validation error：
  `typedJson supports only unary handler returns; HTTP streaming requires rawHttp + Stream<std.http.HttpResponseStreamEvent>`。
- 该分支不调用`ExactTypeClassifier::project_exact(item)`，因此拒绝只取决于
  `typedJson`与外层`Stream<T>`，不会先投影item external schema，也不会构造
  `GatewayDispatchMode::ServerStream`。
- `rawHttp`分支未改变：unary仍要求exact compiler-owned `std.http.HttpResponse`；stream仍要求
  exact compiler-owned `Stream<std.http.HttpResponseStreamEvent>`，并继续生成唯一合法的HTTP
  `ServerStream` surface。nullable、错误owner和其它stream item仍fail closed。

## 3. Direct integration evidence

- 删除F357中的`typedJson + Stream<Output>`正例、对应entry和server-stream断言；保留raw unary与
  raw `Stream<std.http.HttpResponseStreamEvent>`正例。
- `typed_json_streams_fail_before_item_schema_projection`分别使用schema-eligible `Output`和external
  schema不可投影的`Map<string, string>`作为stream item。两者得到完全相同的结构化handler错误，
  证明拒绝不依赖item schema。
- 成功fixture逐个枚举所有`GatewayAdapterKind::TypedJson` surface，证明它们全部为`Unary`、
  `response_schema`存在且`stream_item_schema`为空。
- 既有raw stream item错配负例继续要求`HttpResponseStreamEvent`并通过。

## 4. Reverse search

执行：

```text
rg -n 'GatewayAdapterKind::TypedJson' compiler -g '*.rs'
rg -n 'GatewayDispatchMode::ServerStream' compiler -g '*.rs'
rg -n 'typedJson.*ServerStream|ServerStream.*typedJson|TypedJson.*ServerStream|ServerStream.*TypedJson' \
  compiler -g '*.rs'
```

前两条列出全部单项命中；第三条为零匹配。同一Rust函数内同时包含
`GatewayAdapterKind::TypedJson`与`GatewayDispatchMode::ServerStream`的命中逐项分类如下：

1. Production `compiler/driver/http_gateway_projection/mod.rs::project_handler_return`：
   `TypedJson`的stream arm只返回上述错误；唯一`ServerStream`构造位于相邻`RawHttp` arm；
   `TypedJson`的非stream arm只构造`Unary`。
2. Test
   `compiler/tests/http_gateway_projection.rs::private_http_entries_project_typed_and_raw_unary_plus_raw_stream_without_contract_operations`：
   `TypedJson`只用于筛选并断言全部成功surface为`Unary`且无stream item schema；
   `ServerStream`只断言`rawStream`正例。

其它`TypedJson`命中只属于body/schema校验、service authoring parser测试或不含
`ServerStream`的负例；其它`ServerStream`命中为上述raw production branch与raw test assertion。
`compiler/tests/generated_service_deployment.rs`无二者同函数命中。没有剩余fixture、断言或生产分支把
`typedJson`作为合法`ServerStream`来源。

## 5. Verification

Selector先枚举并确认非零：

| selector | 枚举结果 |
| --- | --- |
| `skiff-compiler --test http_gateway_projection` | 8 tests |

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler --test http_gateway_projection -- --list` | PASS；8 tests，非零 |
| `cargo test -p skiff-compiler --test http_gateway_projection` | PASS；8/8 |
| `cargo test -p skiff-compiler --test generated_service_deployment` | PASS；10/10 |
| `cargo check -p skiff-compiler` | PASS；仅仓库既有warning |
| `rustfmt --edition 2021 --check compiler/driver/http_gateway_projection/mod.rs compiler/tests/http_gateway_projection.rs` | PASS |
| `git diff --check` | PASS |
