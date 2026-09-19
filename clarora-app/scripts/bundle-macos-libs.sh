#!/bin/bash
# 把 Homebrew 动态库闭包打进 Clarora.app,使发布包在未安装 Homebrew 的
# macOS 上可直接启动(否则主程序以绝对路径链接 libmpv/whisper/ggml,
# dyld 在启动时解析失败,双击即崩)。
#
# 用法: scripts/bundle-macos-libs.sh <path-to-Clarora.app>
# 步骤: 递归收集非系统 dylib → 拷入 Contents/Frameworks → install name
# 改写为 @rpath → 移除开发用 rpath → ad-hoc 重签名 → 结构自检。
set -euo pipefail

APP="${1:?usage: bundle-macos-libs.sh <path-to-Clarora.app>}"
BIN="$APP/Contents/MacOS/Clarora"
FRAMEWORKS="$APP/Contents/Frameworks"
# @rpath 依赖的候选解析目录(Homebrew 默认前缀、ASR 脚本安装前缀)。
SEARCH_DIRS=("/opt/homebrew/lib" "/usr/local/lib")

mkdir -p "$FRAMEWORKS"

resolve() { # 把 otool -L 的依赖引用解析为磁盘路径;@rpath 按候选目录查找
    local ref="$1"
    case "$ref" in
        @rpath/*)
            local name="${ref#@rpath/}" dir
            if [ -f "$FRAMEWORKS/$name" ]; then echo "$FRAMEWORKS/$name"; return; fi
            for dir in "${SEARCH_DIRS[@]}"; do
                [ -f "$dir/$name" ] && { echo "$dir/$name"; return; }
            done
            ;;
        *) [ -f "$ref" ] && echo "$ref" ;;
    esac
}

is_system() {
    case "$1" in
        /usr/lib/*|/System/*) return 0 ;;
        *) return 1 ;;
    esac
}

# 1) 递归收集闭包:每个第三方 dylib 拷入 Frameworks 并把自身 ID 改为 @rpath。
declare -a QUEUE=()
while IFS= read -r dep; do
    is_system "$dep" || QUEUE+=("$dep")
done < <(otool -L "$BIN" | awk 'NR>1 {print $1}')

while [ ${#QUEUE[@]} -gt 0 ]; do
    ref="${QUEUE[0]}"
    QUEUE=("${QUEUE[@]:1}")
    resolved="$(resolve "$ref")" || true
    [ -n "${resolved:-}" ] || { echo "WARN: 无法解析依赖 $ref(运行时该功能或不可用)"; continue; }
    name="$(basename "$resolved")"
    dest="$FRAMEWORKS/$name"
    if [ ! -f "$dest" ]; then
        cp -L "$resolved" "$dest"
        chmod 644 "$dest"
        install_name_tool -id "@rpath/$name" "$dest"
        while IFS= read -r sub; do
            is_system "$sub" || QUEUE+=("$sub")
        done < <(otool -L "$dest" | awk 'NR>1 {print $1}')
    fi
done

count=$(ls -1 "$FRAMEWORKS" | wc -l | tr -d ' ')
echo "已打包 $count 个动态库到 Contents/Frameworks"

# 2) 改写主程序与所有已打包 dylib 的依赖引用为 @rpath,并移除开发 rpath。
rewrite() {
    local target="$1" dep name
    while IFS= read -r dep; do
        is_system "$dep" && continue
        name="$(basename "$dep")"
        if [ -f "$FRAMEWORKS/$name" ]; then
            install_name_tool -change "$dep" "@rpath/$name" "$target"
        fi
    done < <(otool -L "$target" | awk 'NR>1 {print $1}')
    # 移除指向开发环境的 rpath(@executable_path/Frameworks 保留)
    while IFS= read -r old; do
        install_name_tool -delete_rpath "$old" "$target" 2>/dev/null || true
    done < <(otool -l "$target" | awk '/LC_RPATH/{getline; getline; print $2}' | grep -v '@executable_path' || true)
}
rewrite "$BIN"
for dylib in "$FRAMEWORKS"/*.dylib; do rewrite "$dylib"; done
install_name_tool -add_rpath "@executable_path/../Frameworks" "$BIN" 2>/dev/null || true

# 3) ad-hoc 重签名(修改过的 dylib 原签名已失效;分发签名在接入 Developer
# ID 后替换)。
codesign --force --sign - "$FRAMEWORKS"/*.dylib 2>/dev/null || true
codesign --force --deep --sign - "$APP"

# 4) 结构自检:主程序与所有 dylib 不得残留绝对第三方路径;所有 @rpath
# 依赖必须能在 Frameworks 内解析。
fail=0
audit() {
    while IFS= read -r dep; do
        is_system "$dep" && continue
        case "$dep" in
            @rpath/*)
                name="${dep#@rpath/}"
                [ -f "$FRAMEWORKS/$name" ] || { echo "AUDIT FAIL: $dep 无法在包内解析"; fail=1; }
                ;;
            *) echo "AUDIT FAIL: 残留绝对路径依赖 $dep"; fail=1 ;;
        esac
    done < <(otool -L "$1" | awk 'NR>1 {print $1}')
}
audit "$BIN"
for dylib in "$FRAMEWORKS"/*.dylib; do audit "$dylib"; done

if [ "$fail" -eq 0 ]; then
    echo "打包完成:第三方动态库已全部内置,脱离 Homebrew 可启动。"
else
    echo "自检未通过,请检查上方 AUDIT FAIL。"
    exit 1
fi
