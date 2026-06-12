#!/bin/bash
# tester-app (Tauri + Vue3) 开发模式启动器（双击运行）
cd "$(dirname "$0")" || exit 1
export PATH="$HOME/.local/bin:$PATH"
source "$HOME/.cargo/env"

npm run tauri dev
