# P5-D06：Canonical Request Raw / Normalization Bounded Audit

## 角色与输入

由原R02A独立只读Agent在第二次正式FAIL后执行；这次审计不是第三次verdict。输入为exact candidate
`571549739239ca16b04d09cd7be1716125dc1982` / tree
`971fa7ad9a63c4b2296b0f7b9ae8e164bcbd02ee`、D03/F03A1合同、TS/Rust production binary request decoder
与同一cross-language corpus。不得修改文件、实现consumer或运行live/gate。

## 冻结结论

第三次R02A verdict前，F03A2必须一次关闭以下同类生产范围：

1. raw JSON string只接受Unicode scalar sequence；lone high/low surrogate、错误pair和错序双端拒绝，合法pair
   双端解码为同一scalar。escape-equivalent key在duplicate检查前按decoded key比较。
2. `testEffectDoubles`的`expectRequest`、`response`及任意nested opaque JSON递归使用同一number域：非有限值和
   超出`±Number.MAX_SAFE_INTEGER`的整数拒绝；`-0`、integral exponent与underflow按ECMAScript JSON值归一，
   finite non-integral number保留。Rust复用leaf `skiff-canonical-json`的number normalization，但须另行拒绝
   unsafe i64/u64；TS不得另造不同number语义。
3. 接受集合相同还不够；decoded typed result也必须相同。缺失的HTTP/WS `adapterArgs`、
   `testEffectsEnabled`、`testEffectDoubles`分别materialize为`[]`、`false`、`{}`；canonical TS decoded type与
   真实返回值一致。serializer仍可省略默认值。
4. corpus至少覆盖lone/valid surrogate、decoded duplicate key、invalid UTF-8/control、non-finite/overflow、
   max/above-safe integer、unsafe rounding、negative zero、integral/fraction exponent、underflow及四组
   absent/default deep-equality。fixture只保存raw bytes/value与预期，不实现第三套parser。

审计还确认F03A1新增TS scanner重复解析binary header/JSON，Unicode规则已因此漂移。F03A2应让canonical
request production入口只产生一个strict decoded header，并复用已有binary framing职责；不得继续叠加
scanner + `JSON.parse`的双解析或修改legacy request接受集合。

## 完成态

输出上述矩阵、最小counterexample、冻结accept/reject/normalize预期及owner；核对candidate HEAD/tree/clean、
activation/store seam未改。审计完成后只解锁F03A2，不解锁F03B/F03C。
