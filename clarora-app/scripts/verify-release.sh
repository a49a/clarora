#!/bin/bash
# 安装包能力验收:检查构建产物是否包含声明的能力所需文件。
# 用法: scripts/verify-release.sh <path-to-Clarora.app> [path-to-Clarora-windows-x64.zip]
# macOS 检查主二进制链接的动态库是否全部内置或为系统库;
# Windows 检查包内是否包含端侧转写与 PDF 运行时。
set -euo pipefail

APP="${1:?用法: verify-release.sh <Clarora.app 路径> [Clarora-windows-x64.zip 路径]}"
ZIP="${2:-}"

failures=0
fail() { echo "✗ $1"; failures=$((failures + 1)); }
pass() { echo "✓ $1"; }

# ── macOS .app 检查 ──────────────────────────────────────────────────────
if [ -d "$APP" ]; then
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
if [ -n "$ZIP" ] && [ -f "$ZIP" ]; then
  # 必须包含的文件:主程序 + 端侧转写 + PDF 运行时
  for required in clarora_asr.dll Clarora.pdf Pdfium.dll; do
    if unzip -l "$ZIP" | grep -qi "$required"; then
      pass "zip 包含 $required"
    else
      fail "zip 缺少 $required(声明能力所需)"
    fi
  done
  # 主程序
  if unzip -l "$ZIP" | grep -qi "Clarora.exe"; then
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
