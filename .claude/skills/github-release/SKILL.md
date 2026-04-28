---
name: github-release
description: 发布 GitHub Release，从 CHANGELOG 生成发布公告并更新 Draft Release (project)
version: 1.0.0
author: https://github.com/BenedictKing/kiro.rs/
allowed-tools: Bash, Read
context: fork
---

# GitHub Release 发布技能

## 触发条件

当用户输入包含以下关键词时触发：

- "发布公告"、"发布说明"、"release notes"
- "发布 release"、"publish release"
- "更新 draft"、"编辑 release"

## 执行步骤

### 1. 获取最新 tag 和检查所有 Draft Release

```bash
# 获取最新 tag
git describe --tags --abbrev=0

# 获取所有 tag 列表
git tag --sort=-v:refname | head -10

# 获取所有 release 列表（包含 draft 状态）
gh release list --limit 10
```

**多 Draft 处理策略**：

- 如果存在多个 Draft Release，只发布最新版本
- 删除中间版本的 Draft Release（快速迭代场景下的合理做法）
- 合并所有中间版本的 changelog 到最新版本的发布公告

### 2. 清理中间版本的 Draft Release

如果检测到多个 Draft：

```bash
# 列出所有 draft release
gh release list --limit 20 | grep -i draft

# 删除中间版本的 draft（保留最新的）
gh release delete <old-tag> --yes
```

**注意**：删除 draft 不会删除对应的 git tag，只是移除 GitHub Release 页面的条目。

### 3. 获取版本间的变更日志

```bash
# 从 CHANGELOG.md 中提取相关版本的内容
cat CHANGELOG.md
```

解析 CHANGELOG.md，提取从上次**公开发布**版本到当前版本的所有变更内容。

### 4. 生成发布公告

根据 CHANGELOG 内容生成简洁的发布公告。

> ⚠️ **【必须】发布公告格式要求**：
>
> 1. 必须按类型分组（✨ 新功能 / 🐛 修复 / 🔧 改进）
> 2. 如果某个分组没有实际内容，**直接忽略该分组**，不要输出占位文案
> 3. **禁止**输出“本版本无新增功能”“无修复”“无改进”等空内容提示
> 4. **必须在末尾包含 Full Changelog 链接**（从上次公开发布版本到最新版本）
> 5. Full Changelog 链接前必须加 `---` 分隔线

**标准格式**：

```markdown
### ✨ 新功能

- 功能点 1
- 功能点 2

### 🐛 修复

- 修复点 1
- 修复点 2

### 🔧 改进

- 改进点 1

---

**Full Changelog**: https://github.com/BenedictKing/kiro.rs/compare/v1.0.0...v1.0.1
```

**内容精简规则（重要）**：

发布公告面向最终用户，必须移除技术实现细节，只保留用户可感知的变化：

| 应移除的内容                              | 应保留的内容                  |
| ----------------------------------------- | ----------------------------- |
| 具体文件路径（`src/anthropic/converter.rs`） | 功能名称                      |
| 代码结构（`CredentialEntry` 结构体）      | 问题现象（返回 403）          |
| 字段名称（`metadata` 字段）               | 用户操作（配置选项）          |
| 实现方式（JSON 反序列化）                 | 修复结果                      |

### 5. 更新 Draft Release 并发布

```bash
# 编辑 release 内容并发布
gh release edit <tag> \
  --title "<tag>" \
  --notes "发布公告内容" \
  --draft=false
```

或者如果没有 draft，直接创建：

```bash
gh release create <tag> \
  --title "<tag>" \
  --notes "发布公告内容" \
  --latest
```

### 6. 确认发布成功

```bash
gh release view <tag> --json url,publishedAt
```

输出发布链接供用户确认。

## 输出格式

> ⚠️ **【必须】严格遵循以下规则输出**
>
> - 版本、状态、链接、发布内容、Full Changelog 不可省略
> - `✨ 新功能 / 🐛 修复 / 🔧 改进` 作为标准样板保留，但**实际输出时空分组可忽略**

```
📦 Release 发布完成！

版本: v1.0.1
状态: ✅ 已发布
链接: https://github.com/BenedictKing/kiro.rs/releases/tag/v1.0.1

已清理的 Draft: v1.0.0（已合并到 v1.0.1 发布公告）

发布内容:
---
### ✨ 新功能
- 功能点

### 🐛 修复
- 修复点

### 🔧 改进
- 改进点

---

**Full Changelog**: https://github.com/BenedictKing/kiro.rs/compare/v1.0.0...v1.0.1
---
```

## 注意事项

- 确保 `gh` CLI 已登录并有仓库权限
- 发布前会显示完整公告内容供用户确认
- 支持多版本合并发布
- 多个 Draft 时只发布最新版本，删除中间版本的 Draft
- 删除 Draft 不影响 git tag，仅清理 GitHub Release 页面
- GitHub 仓库地址: `BenedictKing/kiro.rs`
