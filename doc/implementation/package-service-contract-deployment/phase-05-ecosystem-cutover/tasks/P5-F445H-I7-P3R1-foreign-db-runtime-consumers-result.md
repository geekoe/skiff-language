# P5-F445H-I7-P3R1 Foreign DB runtime consumers result

状态：

```text
PASS
P3R1_COMPLETE = YES
TASK_SCOPE_EXPANDED = NO
DECISION_REQUIRED = NO
```

| 项 | 值 |
| --- | --- |
| implementation commit | `fb8f004e6c57c0b02aa313079a658aaf739843d3` |
| implementation tree | `4925c0be712d19080101b1bad9b9ca326ad73756` |
| baseline | `2b3c0d2cc959d7745ad670a672e5bb29e9d48c23` / `64ab1bd9155fb0929a862f77e235d01a44561a2b` |

## 1. Outcome

Host现在从已admit的provider File IR生成
`exact PackageArtifactRef + exact artifact-owned FileIrRef + local typeIndex`目标，并把该目标与DB
metadata一同交给能力层。Eval从linked DB operation、query和lease carrier解析同一个exact target，
并把它贯穿到raw/prepared DB调用、transaction内部operation和recoverable plan解析。

service-db只按exact target key选择collection。lease hold同时保存exact target key和用于诊断的
`typeName`；claim、renew、release、read及transaction guard都按exact key匹配。`typeName`不再参与
collection、recoverable或lease身份判定。

## 2. Fail-closed matrix

| 场景 | 结果 |
| --- | --- |
| 两个不同exact target同为`model.Session` | PASS；分别索引、互不覆盖 |
| package artifact ref被替换 | rejected |
| File IR ref被替换、缺失或重复 | rejected |
| type index缺失、越界或与declaration不一致 | rejected |
| exact DB attachment缺失 | rejected |
| raw find/insert/update/upsert/replace/delete/count/exists | 全部按exact key |
| prepared create/read/update/replace | 全部按exact key |
| transaction内部operation | 沿用operation exact target |
| recoverable plan | 只从exact provider declaration生成 |
| lease claim/read/renew/release | 全部按exact key |
| 同名另一target的lease guard | 不匹配当前collection |
| D7 identical diamond | provider build只生成一份metadata |

Host构造target时只接受同文件`LocalType`，或同module、同symbol的exact
`DbObjectSymbol`；不做全图、名称或suffix回退。

## 3. RED to GREEN

真实RED：

```text
cargo test -p skiff-runtime-service-db --locked \
  metadata_keeps_identical_type_names_from_distinct_exact_targets_separate -- --nocapture
```

旧实现因为两个target均命名为`model.Session`而报duplicate type。改为exact target索引后，该测试
及同名target的lookup/lease isolation测试均通过。

最终验证：

- `skiff-runtime-capability-context`：unit `66/66`，doc `2/2`；
- `skiff-runtime-service-db`：`114/114`通过，Mongo测试`1`项按约定ignored；
- `skiff-runtime-eval`：unit `404/404`，integration `4/4 + 5/5 + 6/6`，doc `1/1`；
- `skiff-runtime-host`：unit `331/331`，integration `2/2 + 6/6 + 2/2`，doc通过；
- 四包locked check、workspace rustfmt check和`git diff --check`通过；
- exact resolver、exact lookup、lease isolation、program DB及Host D7 focused tests通过。

一次并发执行四包full suite时，两个未触及代码路径的Host async-stream cancellation时序测试失败；
同一Host full suite单独执行为`331/331`，两个失败项随后顺序重跑均通过。因此该并发抖动不构成
P3R1回归。

## 4. Scope and handoff

没有修改compiler、manifest、linker、linked-program schema、D7 projection、artifact generation或
service boundary。没有运行stable/live/network/Mongo/OAuth/browser，也没有push。

P3R1能力层carrier是runtime-only类型；它镜像P3R0的linked exact identity，但没有新增artifact
DTO或建立capability-context到linked-program的反向依赖。
