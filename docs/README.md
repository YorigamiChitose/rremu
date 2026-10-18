# rremu 核心文档

> 状态：核心审查阶段文档，尚未伴随代码重构。
>
> 审查范围：`rremu-cpu`、`rremu-isa`、`rremu-bus`、`rremu-machine`、
> `rremu-config`、`rremu-devices`、`rremu-loader`。
>
> 不包含：`platforms/`、Linux 配置、前端、`rremu-test`。

## 文档索引

| 文档 | 内容 |
|---|---|
| [`architecture.md`](architecture.md) | 模块边界、依赖方向、状态归属 |
| [`interfaces.md`](interfaces.md) | CPU / Bus / Device / Machine / Config 契约 |
| [`invariants.md`](invariants.md) | 架构不变量与测试映射 |
| [`refactor-plan.md`](refactor-plan.md) | 文件拆分与迁移顺序 |
| [`../../CORE-REVIEW-AND-REFACTOR-PLAN.md`](../../CORE-REVIEW-AND-REFACTOR-PLAN.md) | 全项目规划与函数级处置矩阵 |

## 术语

文档中的规范词：

- **MUST**：实现必须满足；违反视为架构缺陷。
- **MUST NOT**：禁止行为。
- **SHOULD**：推荐；偏离需要在代码注释或提交说明中解释。
- **MAY**：允许的实现选择。

## 契约状态

本目录同时记录两类内容：

1. **当前契约**：描述现有代码实际行为。
2. **目标契约**：计划重构后应满足的行为，标记为 `Target`。

如果两者不一致，文档会明确写出 `Current` / `Target`，避免把计划误认为已实现。

## 基线

```text
S 阶段前基线：20a251e
当前分支基线：main
```

## 原则

```text
状态有唯一 owner
接口说明副作用
错误路径显式化
公共语义先文档化，再重构
先移动结构，后改变行为
```
