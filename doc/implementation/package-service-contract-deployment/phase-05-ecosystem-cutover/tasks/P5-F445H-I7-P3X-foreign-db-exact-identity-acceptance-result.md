# P5-F445H-I7-P3X Foreign DB exact identity acceptance result

状态：`PASS`。

## 1. Verdict

P3 foreign package DB contract通过独立纵向验收。

真实test service source经过compiler、linked program、Host activation、Eval DB operation和
fake DB capability store后，两个分别来自不同package artifact、但都名为
`model.Session`的DB object保持精确身份。每个target各完成一次`exists`读取和一次
`delete`写入，分别命中`first_sessions`与`second_sessions`，没有按显示名或相同File IR
内容发生碰撞。

```text
P3X_ACCEPTED = YES
TASK_SCOPE_EXPANDED = NO
```

## 2. Candidate

| 项 | 值 |
| --- | --- |
| 初始候选 commit | `e1530c6a0bdbc7ee4bf6ef9094de7e9a965a3b9e` |
| 初始候选 tree | `50fba533c5698f06d556274db94eb11f0e3d7be4` |
| frozen RED commit | `63e8a6952043873bd0eec57cba175273972ded0b` |
| frozen RED tree | `6bb64a78ca38930132818085cf771cf5140a03a0` |
| repair integration commit | `47b0a35440dbf796835e099c45052a35acb0cd05` |
| repair integration tree | `ec002dbb4400f519ba689d2d8b01fc83f7d70ad6` |
| validation join commit | `23eb56a76e6cdca09faf26045691b14f02cd6d72` |
| validation join tree | `c10d058311f1baebd700bed7a9fde5673865058d` |

P3X分支只新增test-only纵向验收、module注册和task/result。production repair来自已正式集成的
P3R1B，不由本验收节点实现。

## 3. RED and repair receipt

初始候选上，真实编译与Host activation均成功，并产出两份正确的
`DbProviderTargetMetadata`：

- exact package artifact不同；
- exact artifact-owned File IR ref不同；
- local `typeIndex`均为`0`；
- collection分别为`first_sessions`与`second_sessions`。

请求在任何`DbCapabilityStoreApi`方法被调用之前失败：

```text
InvalidArtifact("DB target type index declaration is ambiguous")
```

真实File IR只有一个类型声明，但它的map key是`Session`，声明内canonical symbol是
`model.Session`。旧Eval resolver直接比较两者并误判。P3R1B修复后，同一条test-only
纵向验收由RED转为`1/1 PASS`，四个fake-store事件精确为：

```text
read.exists  -> first_sessions / Session
write.delete -> first_sessions / Session
read.exists  -> second_sessions / Session
write.delete -> second_sessions / Session
```

## 4. Verification

所有命令均使用`--locked`，并在validation join tree上串行执行。

| 层 | 命令/范围 | 结果 |
| --- | --- | --- |
| 真实纵向 | Host exact full-chain selector | `1/1 PASS` |
| compiler import | `skiff-compiler --test package_imports` | `14/14 PASS` |
| source lowering | `foreign_db_targets::tests` | `3/3 PASS` |
| linked program | full crate | `38/38 PASS` |
| linked timeout integration | full target | `1/1 PASS` |
| linker | full crate | `61/61 PASS` |
| capability-context | full crate + docs | `68/68 PASS` |
| service-db | full crate | `114/114 PASS`，另有`1`个明确要求真实Mongo的既有ignored test |
| Eval | unit、integration、docs | `422/422 PASS` |
| Host | unit、integration、docs | `343/343 PASS` |
| locked checks | 9个相关compiler/runtime crates，含tests | `PASS` |
| formatting | workspace rustfmt check | `PASS` |
| diff hygiene | `git diff --check` | `PASS` |

## 5. Exactness, diamond and negative evidence

- compiler source suite拒绝stale package identity、同名File IR substitution、missing file
  link和missing DB attachment；
- linked program保留两个同名foreign DB target的exact dependency identity，并接受真实
  compiler qualified declaration shape；
- Host完整suite证明D7 identical stateful diamond在任意edge order下只有一个effective
  projection；
- 同一Host suite拒绝same-build diamond的不同projection；
- fake store只按exact target lookup key选择collection，测试不按`module/typeName`猜测。

## 6. Scope and environment

- 未修改artifact schema、wire schema、Cargo dependency或production feature；
- 未访问stable instance、network、MongoDB、OAuth或browser；
- 未push；
- RED和GREEN日志保存在`/tmp/skiff-p3x-logs/`，不是仓库交付物。
