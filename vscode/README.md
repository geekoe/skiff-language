# Skiff VS Code Extension

This extension provides syntax highlighting and language configuration for `.skiff` files.

Included support:

- `.skiff` files using the `source.skiff` grammar
- Skiff declarations: `function`, `type`, `alias`, `interface`, `impl`, `const`, `let`, `test`, and modifiers such as `export`, `native`, and `static`
- Block-oriented syntax including `match`, `catch`, `emit`, `concurrent`, `serial`, `value`, and `timeout(...)` block/value-block modifiers
- Core roots and types such as `std`, `root`, `config`, `Array`, `Map`, `Stream`, `JsonObject`, `Exception`, `std.http.HttpRequest`, `std.http.HttpResponse`, `bool`, `integer`, and `bytes`
- Strings, numbers, duration literals, operators, `//` line comments, and `/* ... */` block comments

The grammar intentionally keeps ordinary business variables visually quiet while giving stronger scopes to declaration boundaries, schema/type references, and runtime control/effect forms such as `timeout`, `concurrent`, `emit`, `throw`, `catch`, and `native`.

Package locally:

```bash
pnpm install
pnpm run type-check
pnpm run test:grammar
pnpm run package
```
