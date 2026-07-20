# P4-F11：Checker String-Camouflage Hardening

## Blocker、输入与边界

R03在exact clean candidate `5ba72734c6aa1616020bf4a5defccf3253d08c65`验收FAIL。production entry与router
retirement均PASS，唯一blocker是execution checker用保留字符串内容的`commentless.includes(anchor)`和文本case搜索
证明真实owner。攻击者可以删除host真实active-route调用、把anchor写进惰性字符串；或在TypeScript template string中
伪造完整`case 'request.start'`拒绝、让真实case走selection，checker仍返回零违规。

权威输入为架构§6.2、§7、§12、§14、§15，P4-T09/T09R/R03合同与R03@`5ba7273` verdict。只修checker source
view、owner/call/case结构匹配、mutation与checker tests；不得修改Rust/TypeScript production，不得用路径白名单、
anchor增加数量或production专用例外掩盖问题。

- 依赖：R03@`5ba7273` FAIL。
- 解锁：R03 retry。
- branch：`codex/p4-f11-checker-string-camouflage`。
- worktree：`/Users/geek/workspace/skiff-p4-f11-checker-camouflage`。
- integration边界：只提交task branch，不merge integration/main、不push。

## 完成态

1. source layer提供可复用lexical/token view，区分identifier、punctuation、keyword、comment与string/char/raw/template
   literal；owner/call检查只能消费代码token。literal内容不能满足identifier/call anchor；扫描需保持文件/行定位稳定。
2. Rust required owner facts按token边界证明真实call/field/method shape，不能用裸substring；相似identifier、注释、普通/
   raw/byte字符串与测试helper均不能伪装active lookup、route target、dispatcher call或legacy fence。
3. TypeScript router rejection按真实`switch`中的`case` string-literal token定位`request.start` case，再在该case代码token中
   证明service guard、stable response.error、send与return顺序以及selection/registry/pending/forward缺失。template/
   ordinary string中的伪case不得被识别，真实case改走selection必须失败。
4. hermetic mutation至少新增host惰性字符串伪三个active anchors、router template string伪完整case两类；覆盖普通/
   raw/template literal与相似identifier。两类必须命中稳定ID，修复前内存探针可复现零违规，修复后不可绕过。
5. 原28项mutation、production zero、verify唯一注册全部保持；不得弱化T07/T08 owner、legacy fence或relay全局反向规则。

## 唯一验证 ownership

```bash
node scripts/check-runtime-execution-boundaries.mjs --self-test
node scripts/check-runtime-execution-boundaries.mjs
node --test scripts/tests/runtime-execution-boundary-checker.test.mjs
node scripts/verify.mjs --only checks --list
git diff --check
```

另用R03描述的两个in-memory mutation做独立复现，并运行所有改动模块`node --check`。每个filter/mutation必须非空；
production checker必须零违规。

## 回报

提交一个clean commit，回报token/literal模型、host call与router case结构证明、mutation稳定ID、修复前后结果、合同命令、
extra-review与hash。若可靠处理Rust raw string或JS template interpolation需要完整parser，先回报精确语法缺口，不得退回
substring或仅删除引号内容。
