# rremu 核心文档

本目录描述 rremu 的模块边界、核心接口和架构不变量。

## 文档索引

| 文档 | 内容 |
|---|---|
| [`architecture.md`](architecture.md) | 模块边界、依赖方向、状态归属 |
| [`interfaces.md`](interfaces.md) | CPU / Bus / Device / Machine / Config 接口 |
| [`invariants.md`](invariants.md) | 架构不变量与测试映射 |

## 规范词

- **MUST**：实现必须满足；违反视为架构缺陷。
- **MUST NOT**：禁止行为。
- **SHOULD**：推荐；偏离需要在代码注释或提交说明中解释。
- **MAY**：允许的实现选择。

## 原则

```text
状态有唯一 owner
接口说明副作用
错误路径显式化
公共语义由文档和测试共同锁定
```
