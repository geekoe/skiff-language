# P5-F445H-I7-P3R1 Foreign DB runtime consumers

状态：`IMPLEMENTATION_COMPLETE`。

## 1. Purpose

P3R0已经把每个linked DB target冻结为：

```text
DbObjectTargetId(
  exact PackageArtifactRef,
  exact FileIrRef,
  exact typeIndex
)
```

本节点只迁移Runtime consumers。Host从已admit的provider File IR构造一次exact target→DB
metadata索引；Eval把linked exact target贯穿到能力调用；service-db按exact target选择collection、
recoverable plan和lease guard。`typeName`只保留为诊断显示，不再参与身份判定。

## 2. Baseline and ownership

| 项 | 值 |
| --- | --- |
| baseline commit | `2b3c0d2cc959d7745ad670a672e5bb29e9d48c23` |
| baseline tree | `64ab1bd9155fb0929a862f77e235d01a44561a2b` |
| branch | `codex/p5-f445h-i7-p3r1-db-runtime` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p3r1-db-runtime` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检后才创建上述worktree。预检与实现均未修改compiler、manifest、linker、
linked-model schema、D7 projection规则、artifact generation或service boundary。

## 3. Write scope

允许修改：

- `runtime/host`：activation DB metadata的exact target构造和direct tests；
- `runtime/eval`：DB operation/query/transaction内部operation/lease claim/read的exact target传播，
  以及recoverable declaration的exact解析；
- `runtime/capability-context`：runtime-only exact DB target carrier；
- `runtime/service-db`：exact target索引、prepared/raw operation、transaction lease guard及直接测试；
- 本task/result。

禁止修改：

- compiler/source/driver/lowering与manifest；
- runtime linked model、linker和P3R0 identity schema；
- D7 canonical projection merge；
- File IR、PackageArtifact、Local ABI、ServiceContract或RuntimeAssembly DTO/generation；
- stable instance、live/network/Mongo/OAuth/browser或外部状态。

## 4. Required behavior

### 4.1 Host

- 每个effective provider build只读取一次provider File IR metadata；
- target identity必须包含exact artifact ref、exact artifact-owned file ref和local type index；
- DB declaration必须指向同文件的exact local DB type；
- D7 identical diamond合并后只生成一份该build的metadata；
- missing、duplicate、out-of-bounds或substituted artifact/file/type必须fail closed。

### 4.2 Eval

- `DbOperation`所有find/insert/update/upsert/replace/delete/count/exists分支携带同一个exact target；
- `DbQuery`构造、transaction内部operation、lease claim/read继续携带exact target；
- recoverable field plan只从exact target所指向的provider declaration生成；
- 不扫描全图，不按module/type名或suffix猜测provider。

### 4.3 service-db

- collection只按exact target key查找；
- 同名`model.Session`来自两个不同artifact/file/type index时互不相撞；
- prepared runtime与raw operation使用同一exact lookup；
- lease claim产生的hold保存exact target key；renew/release/read与transaction guard继续按该key；
- 另一个exact target上的同名lease不得为当前collection加guard；
- 缺失或被替换的target必须fail closed。

## 5. Acceptance

- 先用“两个不同exact target、相同`model.Session`”获得真实RED；
- exact lookup、substitution rejection、lease guard isolation与assembly resolver direct tests转GREEN；
- capability-context、service-db、Eval、Host locked full suites通过；
- 四包locked check、workspace rustfmt check、`git diff --check`和禁止fallback反向搜索通过。
