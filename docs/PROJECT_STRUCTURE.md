# 项目结构

```
tester-app/
├── index.html                   # Vite 入口 HTML
├── package.json                 # 前端依赖与脚本
├── package-lock.json            # 依赖锁定文件
├── vite.config.ts               # Vite 开发服务器配置（端口 1420、HMR）
├── tsconfig.json                # TypeScript 编译器配置（严格模式）
├── tsconfig.node.json           # Vite 配置文件专用 TypeScript 配置
├── dev.bat                      # Windows 快速启动脚本
├── README.md                    # 项目说明
├── src/                         # Vue 3 前端源码
│   ├── main.ts                  # Vue 应用入口，挂载根组件
│   ├── App.vue                  # 根组件：认证状态路由（登录页 ↔ 主页）
│   ├── vite-env.d.ts            # Vite 类型声明
│   ├── assets/                  # 静态资源
│   │   └── vue.svg              # Vue logo
│   └── pages/                   # 页面组件
│       ├── LoginPage.vue        # Google OAuth 登录页
│       ├── MainPage.vue         # 主布局：三级导航（工作区 → 选项 → 内容）
│       ├── SheetsPage.vue       # Google Sheets 浏览与选择
│       ├── SlidesPage.vue       # Google Slides 浏览与多页选择
│       ├── GeneratePage.vue     # Claude 测试用例生成（导出 + 流式日志 + 多轮对话）
│       └── SettingsPage.vue     # 缓存管理设置
├── public/                      # 公共静态资源
│   ├── tauri.svg                # Tauri logo
│   └── vite.svg                 # Vite logo
├── src-tauri/                   # Rust 后端（Tauri 2）
│   ├── Cargo.toml               # Rust 依赖声明
│   ├── Cargo.lock               # Rust 依赖锁定
│   ├── build.rs                 # Tauri 构建脚本
│   ├── tauri.conf.json          # Tauri 应用配置（窗口、标识符、CSP、构建命令）
│   ├── src/                     # Rust 源码
│   │   ├── main.rs              # 程序入口（最小化，调用 lib）
│   │   ├── lib.rs               # Tauri 应用初始化与命令注册
│   │   ├── auth.rs              # Google OAuth 2.0 + PKCE 认证、令牌刷新与持久化
│   │   ├── sheets.rs            # Google Drive / Sheets / Slides API 封装与导出
│   │   └── claude.rs            # Claude CLI 子进程管理、流式 JSON 解析与会话续接
│   ├── capabilities/            # Tauri 权限能力
│   │   └── default.json         # 默认权限：core:default、opener:default
│   ├── credentials/             # OAuth 凭据（已 gitignore）
│   │   └── oauth.json           # Google OAuth client_id / client_secret
│   └── icons/                   # 应用图标（多平台多尺寸）
│       ├── icon.png             # 通用 PNG
│       ├── icon.ico             # Windows ICO
│       ├── icon.icns            # macOS ICNS
│       ├── 32x32.png            # 32px
│       ├── 128x128.png          # 128px
│       ├── 128x128@2x.png      # 128px Retina
│       └── Square*.png          # Windows Store 各尺寸
└── docs/                        # 项目文档
    └── PROJECT_STRUCTURE.md     # 本文件
```

# 运行时数据目录

所有持久化数据存储在用户主目录下的 `~/.tester-app/`：

```
~/.tester-app/
├── auth-tokens.json             # OAuth access_token / refresh_token / 过期时间
├── auth-user.json               # 用户信息（email、name、picture）
├── exports/                     # 导出文件
│   ├── sheet_*.csv              # Google Sheets CSV 导出
│   └── *.pptx                   # Google Slides PPTX 导出
└── thumbs/                      # 幻灯片缩略图缓存
    └── {presentation_id}/
        ├── 1.png
        ├── 2.png
        └── ...
```

# 核心模块说明

## 前端页面

| 页面 | 文件 | 职责 |
|---|---|---|
| 登录页 | `LoginPage.vue` | Google 账号登录，调用后端 `start_login()` 启动 OAuth 流程 |
| 主布局 | `MainPage.vue` | 三级导航容器（工作区栏 → 选项栏 → 内容区），管理页面间数据传递 |
| Sheets 页 | `SheetsPage.vue` | 双栏布局：左侧文件列表 + URL 粘贴，右侧数据预览与 Tab 切换 |
| Slides 页 | `SlidesPage.vue` | 三栏布局：文件列表 + 大图预览 + 缩略图条，支持多页勾选 |
| 生成页 | `GeneratePage.vue` | 展示选择摘要 → 一键导出并调用 Claude → 终端风格日志 → 多轮对话输入 |
| 设置页 | `SettingsPage.vue` | 缓存大小查看与清理 |

## 后端模块

| 模块 | 文件 | 职责 |
|---|---|---|
| 认证 | `auth.rs` | Google OAuth 2.0 PKCE 流程：本地 TCP 回调服务器、令牌交换、自动刷新、持久化至文件 |
| Google API | `sheets.rs` | Drive 文件列表、Sheets 读取与 CSV 导出、Slides 幻灯片获取与 PPTX 导出、缩略图异步缓存 |
| Claude 集成 | `claude.rs` | 定位 Claude CLI 路径、子进程管理、stream-json 输出解析、会话 ID 续接、实时事件推送 |

# 依赖库

## 前端依赖（npm）

| 库 | 类型 | 作用 |
|---|---|---|
| `vue` | 核心 | Vue 3 响应式 UI 框架 |
| `@tauri-apps/api` | 核心 | Tauri 前后端通信桥接（invoke 命令、事件监听） |
| `@tauri-apps/plugin-opener` | 插件 | 在系统浏览器中打开 URL |
| `typescript` | 开发 | 静态类型检查 |
| `vite` | 开发 | 前端构建与开发服务器 |
| `@vitejs/plugin-vue` | 开发 | Vite 的 Vue SFC 编译插件 |
| `vue-tsc` | 开发 | Vue 模板的 TypeScript 类型检查 |

## 后端依赖（Cargo）

| 库 | 类型 | 作用 |
|---|---|---|
| `tauri` | 核心 | Tauri 2 桌面应用框架 |
| `tauri-build` | 构建 | Tauri 构建时脚本 |
| `tauri-plugin-opener` | 插件 | 打开 URL / 文件的 Tauri 插件 |
| `tokio` | 运行时 | 异步运行时（net、io-util、process） |
| `reqwest` | 网络 | HTTP 客户端，用于 Google API 调用和缩略图下载 |
| `serde` / `serde_json` | 序列化 | JSON 序列化与反序列化 |
| `sha2` | 加密 | PKCE code_challenge 的 SHA-256 哈希 |
| `base64` | 编码 | PKCE code_verifier 的 Base64 URL 编码 |
| `rand` | 工具 | PKCE 随机字节生成 |
| `dirs` | 工具 | 跨平台用户主目录路径获取 |
| `open` | 工具 | 在默认浏览器中打开 OAuth 授权页面 |
| `urlencoding` | 工具 | URL 编码处理 |

# 数据流

```
登录认证
  └─ LoginPage → start_login() → 浏览器 OAuth 授权 → 本地回调 → 令牌交换 → 持久化 → 进入主页

选择测试数据
  ├─ SheetsPage: 文件列表 / URL 粘贴 → 获取 Tab → 预览数据 → 确认选择
  └─ SlidesPage: 文件列表 / URL 粘贴 → 加载幻灯片 → 异步缩略图 → 勾选页面

生成测试用例
  └─ GeneratePage:
       ├─ 导出 Sheet CSV  → export_sheet_csv()  → ~/.tester-app/exports/sheet_*.csv
       ├─ 导出 Slides PPTX → export_slides_pptx() → ~/.tester-app/exports/*.pptx
       └─ 调用 Claude CLI  → run_claude_task(csv, pptxs, pages)
            └─ claude --print --verbose --output-format stream-json --file <csv> --file <pptx> '/test-case-generator ...'
                 └─ 流式 JSON → claude-log 事件 → 前端终端日志

多轮对话
  └─ 用户输入 → send_claude_input(input) → claude --resume <session_id> → 流式响应
```
