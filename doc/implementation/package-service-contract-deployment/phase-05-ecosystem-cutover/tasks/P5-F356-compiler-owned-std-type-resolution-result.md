# P5-F356 Compiler-owned std type-resolution owner result

状态：Completed（独立source-owner follow-up；未运行workspace/root、stable/live，未push）。

## Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| task base | `c7f285089817f692429aecab0ad03de2cea3d78d` | `c3d84a10f212ffd4ae2c98c0dd59c1c30b07595d` |
| production/tests | `482b92f310f64dce0b9dbc95b1643afd8d2c674b` | `64bf05f7cf0713018f1c3ffa895b2a495243d5bc` |

工作分支为`codex/p5-f356-std-owner`，worktree为
`/Users/geek/workspace/skiff-p5-f356-std-owner`。没有merge/rebase integration，没有操作
stable instance或live配置。

## Outcome

- `canonical_dependencies`现在只选择一次唯一且identity-valid的canonical
  `skiff.run/std`。同一选择同时生成compiler-owned source dependency facts，并把该exact
  artifact加入type-resolution artifact集合；其它`available_packages`不会跨过该边界。
- compiler-owned owner facts携带exact Package build ID与Local ABI identity。
  `TypeResolutionModel`只允许这两个identity同时匹配且恰好匹配一个artifact时建立alias、
  package ID、type slot、source path、constant及lowering ABI owner；zero/multiple match与声明
  dependency alias冲突都会fail closed。
- compiler-owned artifact只按其完整public path建立type索引，不引入short-name猜测，也没有新增
  `std.websocket`名称特例。Exact callable signature中的Local ABI slot与generic source type均可
  rehydrate并lower。
- 用户仍无需在`package.yml`声明`std`。authoring/public wire、PackageArtifact schema、
  ServiceContract及gateway/deployment/runtime/router语义均未改变。
- 真实回归package同时携带code-free service contract dependency并调用
  `std.time.sleep`、`std.websocket.sendTextToConnection`；输出只有一个`std` package
  requirement和一个contract requirement，没有provider package泄漏。

## Validation evidence

| Command | Result |
| --- | --- |
| `cargo test -p skiff-compiler-source compiler_owned -- --list` | PASS；5 tests listed，非零 |
| `cargo test -p skiff-compiler compiler_owned_std -- --list` | PASS；4 tests listed，非零 |
| `cargo test -p skiff-compiler-source compiler_owned` | PASS；5/5 |
| `cargo test -p skiff-compiler compiler_owned_std` | PASS；4/4（2 lib + 2 real integration） |
| `cargo test -p skiff-compiler public_generic` | PASS；2/2 |
| `cargo test -p skiff-compiler service_call` | PASS；4 selected tests |
| `cargo check -p skiff-compiler-source -p skiff-compiler` | PASS；仅既有unused/dead-code warnings |
| `cargo fmt -p skiff-compiler-source -p skiff-compiler-input-model -p skiff-compiler -- --check` | PASS |
| `git diff --check` | PASS |

## Negative owner and reverse-scan evidence

- `compiler_owned_std_selection_is_exact_and_fail_closed`覆盖唯一std选择、unrelated non-std
  available artifact不被选择、absence不伪造owner、duplicate拒绝与forged identity拒绝。
- `compiler_owned_package_owner_requires_one_exact_artifact`覆盖required owner的zero/multiple exact
  artifact match均拒绝。
- `compiler_owned_available_artifacts_require_explicit_owner_facts`证明仅出现在artifact候选集、但没有
  compiler-owned owner facts的artifact不会得到alias、identity或type owner。
- 新增production diff中没有`available_packages.to_vec()`或把全部available artifact直接传给
  source resolution的路径；只有canonical std selector读取`available_packages`。
- 新增production diff中没有`std.websocket`匹配或名称分支。仓库中既有的structured boundary
  unavailable分类保持不变。

## Self-acceptance matrix

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| analysis、type slots与lowering使用同一exact std owner | 单次canonical selector；owner以build ID + Local ABI双identity join | 没有第二个std重选函数或alias-only artifact lookup | exact signature、generic source type与lowered requirement integration tests |
| 其它available package不进入source resolution | type-resolution集合仅为declared artifacts加已选std | 无whole-`available_packages`复制/透传 | unrelated available与missing explicit owner negative tests |
| std无需用户声明且contract保持code-free | compiler-owned facts独立于manifest dependencies；contract input未改 | 无authoring/wire或ServiceContract schema改动 | real package同时含contract dependency、sleep与WebSocket call |
| missing/duplicate/wrong/ambiguous fail closed | selector identity validation；owner要求exactly one match | 无fallback、first-match或alias猜测 | duplicate、forged identity、zero/multiple owner tests |
| F353/F352语义不回归 | 完整type path indexing，不改schema eligibility/root selection | 无新增`std.websocket`production branch | `public_generic`与`service_call`focused selectors全绿 |

## Scope

改动仅位于compiler source dependency/type-resolution接线、内部input说明和compiler tests。
没有修改F353 schema eligibility、PackageArtifact/ServiceContract schema、gateway/deployment、
runtime/router/test-runner、lockfile或三仓库service源码；没有运行workspace/root、stable/live，
没有push。
