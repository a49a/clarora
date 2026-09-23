#!/bin/bash
# 安装包能力验收:检查构建产物是否包含声明的能力所需文件。
# 用法: scripts/verify-release.sh [Clarora.app 路径] [Clarora-windows-x64.zip 路径]
# macOS 检查主二进制链接的动态库是否全部内置或为系统库;
# Windows 检查包内是否包含端侧转写与 PDF 运行时。两个输入至少提供一个。
set -euo pipefail

APP=""
ZIP=""
for arg in "$@"; do
  case "$arg" in
    *.zip) ZIP="$arg" ;;
    *) APP="$arg" ;;
  esac
done

if [ -z "$APP" ] && [ -z "$ZIP" ]; then
  echo "用法: verify-release.sh [Clarora.app 路径] [Clarora-windows-x64.zip 路径]"
  exit 1
fi

# 输入缺失立即失败,不允许静默跳过全部检查后仍报通过
if [ -n "$APP" ] && [ ! -d "$APP" ]; then
  echo "✗ 输入不存在或不是 .app 目录: $APP"
  exit 1
fi
if [ -n "$ZIP" ] && [ ! -f "$ZIP" ]; then
  echo "✗ 输入不存在或不是文件: $ZIP"
  exit 1
fi

failures=0
fail() { echo "✗ $1"; failures=$((failures + 1)); }
pass() { echo "✓ $1"; }

# ── 归档工具:Windows Git Bash 可能没有 unzip,退回系统自带 bsdtar ──
if command -v unzip >/dev/null 2>&1; then
  list_archive() { unzip -l "$1" | awk 'NR>3 && NF>=4 {print $4}'; }
  extract_archive() { unzip -qq -o "$1" -d "$2"; }
elif [ -x /c/Windows/System32/tar.exe ]; then
  list_archive() { /c/Windows/System32/tar.exe -tf "$1"; }
  extract_archive() { mkdir -p "$2"; /c/Windows/System32/tar.exe -xf "$1" -C "$2"; }
else
  list_archive() { tar -tf "$1"; }
  # tar 不会自动创建目标目录,必须先建,否则嵌套包解压直接失败
  extract_archive() { mkdir -p "$2"; tar -xf "$1" -C "$2"; }
fi

# ── macOS .app 检查 ──────────────────────────────────────────────────────
if [ -n "$APP" ]; then
BIN="$APP/Contents/MacOS/Clarora"
  FW="$APP/Contents/Frameworks"

  # 主二进制存在且可执行
  [ -x "$BIN" ] || { fail "主二进制不存在或不可执行: $BIN"; }

  # JS bundle 存在(离线运行)
  JSB="$APP/Contents/Resources/main.jsbundle"
  [ -f "$JSB" ] || fail "JS bundle 缺失: $JSB"

  # 所有第三方动态库已内置(复审问题:绝对路径 dylib 导致启动崩溃)
  abs_deps=0
  while IFS= read -r dep; do
    case "$dep" in
      /usr/lib/*|/System/*|@rpath/*|@loader_path/*|@executable_path/*) ;;
      *) abs_deps=$((abs_deps + 1)); echo "  绝对路径依赖: $dep" ;;
    esac
  done < <(otool -L "$BIN" | awk 'NR>1 {print $1}')
  [ "$abs_deps" -eq 0 ] || fail "主二进制有 $abs_deps 个非系统绝对路径依赖"

  # Frameworks 目录中的 dylib 同样不得有绝对路径依赖
  if [ -d "$FW" ]; then
    for dylib in "$FW"/*.dylib; do
      [ -f "$dylib" ] || continue
      bad=0
      while IFS= read -r dep; do
        case "$dep" in
          /usr/lib/*|/System/*|@rpath/*|@loader_path/*|@executable_path/*) ;;
          *) bad=$((bad + 1)) ;;
        esac
      done < <(otool -L "$dylib" | awk 'NR>1 {print $1}')
      [ "$bad" -eq 0 ] && continue
      fail "$(basename "$dylib") 有 $bad 个非系统绝对路径依赖"
    done
  fi

  # ad-hoc 签名有效
  codesign --verify --deep --strict "$APP" 2>/dev/null || fail "codesign --verify --deep --strict 失败"

[ "$failures" -eq 0 ] && pass "macOS .app 能力检查通过"
fi

# ── Windows .zip 检查 ────────────────────────────────────────────────────
if [ -n "$ZIP" ]; then
  # 发布 zip 的外层只看得到 .msix 应用包名;端侧转写与 PDF 运行时位于嵌套
  # 应用包内部,必须解开再列,否则完整安装包会被误判为缺失。
  listing="$(list_archive "$ZIP")"
  if printf '%s\n' "$listing" | grep -qiE '\.(msix|appx|msixbundle|appxbundle|zip)[[:space:]]*$'; then
    staging="$(mktemp -d)"
    trap 'rm -rf "$staging"' EXIT
    extract_archive "$ZIP" "$staging/outer"
    while IFS= read -r inner; do
      [ -f "$inner" ] || continue
      listing="$listing
$(list_archive "$inner")"
    done < <(find "$staging/outer" -type f \( -iname '*.msix' -o -iname '*.appx' -o -iname '*.msixbundle' -o -iname '*.appxbundle' -o -iname '*.zip' \) 2>/dev/null)
  fi
  # 必须包含的文件:主程序 + 端侧转写 + PDF 运行时(pdfium.dll)
  for required in clarora_asr.dll pdfium.dll; do
    if grep -qi "$required" <<< "$listing"; then
      pass "zip 包含 $required"
    else
      fail "zip 缺少 $required(声明能力所需)"
    fi
  done
  # 主程序
  if grep -qi "Clarora.exe" <<< "$listing"; then
    pass "zip 包含 Clarora.exe"
  else
    fail "zip 缺少 Clarora.exe"
  fi
fi

if [ "$failures" -eq 0 ]; then
  echo "安装包能力验收全部通过。"
else
  echo "安装包能力验收有 $failures 项失败。"
  exit 1
fi
