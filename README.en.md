# Clarora — Language Learning Tools

- **Flashcards & Review**: Vocabulary, audio clips, AI Q&A cards; SM-2 spaced repetition; text, directory, and Anki import (Anki deck support on macOS / Windows desktop; mobile import not supported — cards imported on desktop are distributed via Sync & Backup).
- **Audio & Video Learning**: Per-sentence subtitles, playback speed, looping, bilingual subtitle merging, clip favorites, audio management, and native playback.
- **Discover**: Mixed recommendations from local learning content, with favorites and audio clip preloading.
- **Study Tools**: Statistics, Pomodoro timer, meditation, themes, and a flashcard list.
- **Self-Hosted Backup**: Client connects directly to S3-compatible storage or Alibaba Cloud OSS, keeps versions, and merges backups on other devices.
- **Custom AI API**: Chat, streaming responses, keyword highlighting, translation, OCR, transcription, and shadowing alignment scoring. Bring your own model and API key.
- **On-Device Subtitles (macOS / Windows)**: whisper.cpp and SenseVoice local models generate subtitles offline — audio never leaves your machine; English-to-Chinese translation uses the chat API. Models are one-click downloads in Settings.
- **Document Reader (macOS / Windows)**: Built-in PDF reader with sidebar TOC, full-page rendering, and zoom, for offline study materials.

## Platforms

Clarora has two products: the GPUI native desktop app for macOS / Windows, and the React Native mobile app for Android / iOS. Desktop flashcards, audio, and progress can be merged on mobile through Sync & Backup.

| Platform | Build | Download / install | Notes |
| --- | --- | --- | --- |
| macOS 14+ Apple Silicon | GPUI desktop | Download `Clarora-macos-arm64.dmg` from [GitHub Releases](https://github.com/a49a/clarora/releases/latest) | Drag it into Applications. If Gatekeeper blocks the first launch, right-click the app and choose "Open", or run `xattr -dr com.apple.quarantine /Applications/Clarora.app` |
| Windows 10 1809+ / 11 x64 | GPUI desktop | Prefer `Clarora-windows-x64-setup-unsigned.exe`; portable: `Clarora-windows-x64.zip` | If SmartScreen appears, choose "More info" → "Run anyway"; no Developer Mode or sideloading script is required |
| Android | React Native mobile | Download `Clarora-android.apk` from the website or the matching GitHub Release | Requires a release that actually published the signed APK; desktop audio and flashcards arrive through Sync & Backup |
| iOS 15.1+ | React Native mobile | Use the TestFlight / App Store link on the website; if unavailable, the Release only contains `Clarora-ios-unsigned.ipa` | The unsigned IPA must be signed before installation and cannot be installed as-is; an IPA does not mean TestFlight / App Store is available |

- Website (features, guides, and downloads): <https://a49a.github.io/clarora-web/>
- Desktop installers are unsigned. Speech models are downloaded in the app, not bundled; personal study data stays on this device or in storage you configure.
- The mobile app does not include desktop-only video download, Anki deck import, or directory/command import. See [Mobile Releases](docs/mobile-releases.md) and [Platform Notes](docs/platform-builds.md).
- The RN macOS / Windows desktop builds are deprecated; use the GPUI desktop app above. Android / iOS remain maintained.

First desktop use: open Audio Learning to import local audio, or Video Learning to import a local video; generate subtitles from the subtitle toolbar when none exist. On mobile, open Settings → Sync & Backup to merge a desktop backup; text-card, OCR, and file import depend on the platform's supported scope.

## Development

The RN macOS and Windows desktop builds are deprecated; there is no supported use or build entry for them. Use the GPUI desktop client instead. Android and iOS remain maintained.

GPUI desktop development has one cross-platform launcher:

```sh
npm run gpui
```

It requires Node.js plus Rust 1.93.1. On Windows use the MSVC toolchain, Visual Studio 2022's Desktop development with C++ workload, and the Windows SDK. The launcher automatically calls `clarora-gpui/scripts/dev.sh` on macOS or `dev.ps1` on Windows. macOS can still use `make gpui` without Node; Windows can run `powershell -File clarora-gpui/scripts/dev.ps1` directly.

Requires Node.js 24 plus Android Studio or Xcode:

```sh
npm run setup
npm start
# In a separate terminal, pick a mobile platform:
npm run android
npm run ios
```

See [Platform Notes](docs/platform-builds.md) for Android / iOS build steps and platform limitations. CI in `.github/workflows/` runs shared-code type checking and tests on push and PR, and compiles Android / iOS. Shared code and automated tests are not equivalent to on-device acceptance.

```sh
npm run typecheck
npm test
```

## Sync & Backup

Fill in storage type, Endpoint, Region, Bucket, Access Key, and library prefix under "Settings → Sync & Backup". All devices use the same storage location; one device uploads, another reads versions and merges. No database, API, or task queue to deploy.

The current model is manual versioned backup with additive merge: deletions do not propagate, existing local content wins, review schedules take the later grade, and daily stats take the larger value. Each upload is a full backup, not a real-time bidirectional sync.

Backups include flashcards, audio/video study attachments, subtitles, favorites, review schedules, statistics, OCR and shadowing results. Credentials, device settings, unfavorited recent videos, chat session drafts, raw OCR images, and shadowing recordings are not included. Credentials on macOS / Windows are stored in the system credential vault (Keychain / Credential Manager); mobile platforms store them in the local app database for now; cloud backup uses HTTPS but has no end-to-end encryption.

See [Storage Sync Notes](docs/storage-sync.md) for detailed configuration, permissions, data formats, and restore strategy.

## AI

Configure an OpenAI-compatible Base URL, API Key, and model name under "Settings → AI Services". Chat/translation uses `/chat/completions`; OCR requires a vision model; transcription uses `/audio/transcriptions` with `verbose_json` and `segments` timeline support. Chat and transcription services can be configured independently, or point to a self-hosted compatible endpoint.

Local models trade accuracy for speed. For English, start with `small.en` for most subtitles; `medium.en` is more accurate but slower, so re-transcribe only audio whose subtitles are clearly inaccurate. For Chinese or multilingual audio, use `small`, SenseVoice, or `large-v3-turbo`; `.en` models are English-only.

OCR and shadowing results are stored locally. Shadowing scoring compares recognized text against the reference sentence — it is not an acoustic pronunciation assessment. Without AI configuration, local import, playback, and review still work.

## Support Clarora

If Clarora helps your learning, you can buy me a coffee. Donations are optional and do not affect any feature.

<table>
  <tr>
    <th align="center">Alipay</th>
    <th align="center">WeChat</th>
  </tr>
  <tr>
    <td align="center"><img src="docs/assets/alipay.png" alt="Alipay donation QR code" width="240" /></td>
    <td align="center"><img src="docs/assets/wechat-pay.png" alt="WeChat donation QR code" width="240" /></td>
  </tr>
</table>

## Repository Structure

| Path | Contents |
| --- | --- |
| `clarora-app/` | Four-platform React Native client, shared data layer, object storage, and AI adapters |
| `tool/` | Client icon generation tool |
| `docs/` | Client build and storage sync documentation |

The root LICENSE retains the existing BSD-3-Clause notice. The RN built-in graph uses original sample data from this project; third-party word lists and personal study materials are not distributed with the source code. Licenses for third-party dependencies, model services, libmpv, etc. must be reviewed based on actual distribution content.


Desktop Anki import: on macOS / Windows, choose "Anki Import" on the flashcard page. Supports decks and Anki `.txt` / `.tsv` text files; see [Import Notes](docs/anki-import.md) for formats, merge rules, and limitations.
