# tester-app

Google Play 评论回复工具，基于 Claude CLI 本地驱动。

## 下载安装

前往 [Releases](../../releases) 页面，下载对应系统的安装包：

- **macOS** → 下载 `.dmg` 文件
- **Windows** → 下载 `.exe` 文件

---

## 安装说明

### macOS

由于 app 未经 Apple 公证，首次打开时系统会提示"无法验证开发者"。

**解决方法：**

1. 打开 `.dmg`，将 app 拖入「应用程序」文件夹
2. 打开终端，运行以下命令：
   ```
   xattr -dr com.apple.quarantine /Applications/tester-app.app
   ```
3. 之后正常双击打开即可

---

### Windows

由于安装包未签名，Windows SmartScreen 会弹出蓝色警告页面。

**解决方法：**

1. 在警告页面点击**「更多信息」**
2. 点击**「仍要运行」**
3. 按正常安装步骤完成安装

---

## 前置依赖

本 app 通过本机 Claude CLI 调用模型，使用前请确保：

1. 已安装 Claude CLI：
   ```
   npm install -g @anthropic-ai/claude-code
   ```
2. 已登录：运行 `claude`，按提示完成登录

安装状态可在 app 的 Settings 页面查看。
