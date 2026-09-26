# Clarora — Language Learning Tools

- **Flashcards & Review**: Vocabulary, audio clips, AI Q&A cards; SM-2 spaced repetition; text, directory, and Anki import (Anki deck support on macOS / Windows desktop; mobile import not supported — cards imported on desktop are distributed via Sync & Backup).
- **Audio & Video Learning**: Per-sentence subtitles, playback speed, looping, bilingual subtitle merging, clip favorites, audio management, and native playback.
- **Discover**: Mixed recommendations from local learning content, with favorites and audio clip preloading.
- **Study Tools**: Statistics, Pomodoro timer, meditation, themes, word relationship graph.
- **Self-Hosted Backup**: Client connects directly to S3-compatible storage or Alibaba Cloud OSS, keeps versions, and merges backups on other devices.
- **Custom AI API**: Chat, streaming responses, keyword highlighting, translation, OCR, transcription, and shadowing alignment scoring. Bring your own model and API key.
- **On-Device Subtitles (macOS / Windows)**: whisper.cpp and SenseVoice local models generate subtitles offline — audio never leaves your machine; English-to-Chinese translation uses the chat API. Models are one-click downloads in Settings.
- **Document Reader (macOS / Windows)**: Built-in PDF reader with sidebar TOC, full-page rendering, and zoom, for offline study materials.

## Download

macOS and Windows installers are available from [GitHub Releases](https://github.com/a49a/clarora/releases/latest) starting with v0.1.0:

- macOS: `Clarora-macos-arm64.dmg` (Apple Silicon, runtime dependencies bundled, no Homebrew needed)
- Windows: `Clarora-windows-x64.zip` (extract and sideload per included instructions; Developer Mode required)

Installers are not code-signed: on macOS, right-click the app and choose "Open" if Gatekeeper blocks the first launch; on Windows, click "More info" → "Run anyway" if SmartScreen appears.

For signed mobile builds and distribution, see [Mobile Releases](docs/mobile-releases.md). The Android APK is based on actual Release assets; the iOS Release provides `Clarora-ios-unsigned.ipa` (subject to actual assets), which requires user-side signing before installation — it cannot be installed as-is. Building the unsigned package does not require a paid Apple account.

## Development

Requires Node.js 24 and platform-specific toolchains. macOS additionally needs Xcode, CocoaPods, libmpv, and whisper-cpp (`brew install mpv whisper-cpp`); the project looks for these under `/opt/homebrew` — adjust Xcode Header / Library Search Paths for other install locations.

```sh
npm run setup
npm start
# In a separate terminal, pick a platform:
npm run macos
npm run android
npm run windows
npm run ios
```

See [Platform Notes](docs/platform-builds.md) for Windows / iOS build steps and platform limitations. CI in `.github/workflows/` runs shared-code type checking and tests on push and PR, and compiles Android / macOS / iOS; the Windows package is built on push or can be triggered manually. Shared code and automated tests are not equivalent to on-device acceptance for all four platforms.

`npm run macos` checks local Pods; it runs `pod install` automatically on first build or when the native dependency lockfile changes, so CocoaPods must be installed first. After modifying a Podfile, run `npm run macos:pods` in `clarora-app/` to update dependencies manually. `Pods/` is generated locally and not committed to the repository.

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

OCR and shadowing results are stored locally. Shadowing scoring compares recognized text against the reference sentence — it is not an acoustic pronunciation assessment. Without AI configuration, local import, playback, and review still work.

## Repository Structure

| Path | Contents |
| --- | --- |
| `clarora-app/` | Four-platform React Native client, shared data layer, object storage, and AI adapters |
| `tool/` | Client icon generation tool |
| `docs/` | Client build and storage sync documentation |

The root LICENSE retains the existing BSD-3-Clause notice. The built-in graph uses original sample data from this project; third-party word lists and personal study materials are not distributed with the source code. Licenses for third-party dependencies, model services, libmpv, etc. must be reviewed based on actual distribution content.


Desktop Anki import: on macOS / Windows, choose "Anki Import" on the flashcard page. Supports decks and Anki `.txt` / `.tsv` text files; see [Import Notes](docs/anki-import.md) for formats, merge rules, and limitations.
