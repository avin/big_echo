# BigEcho Implementation Status

## Implemented in this iteration

- Separate project folder `BigEcho`
- Rust app skeleton for Tauri 2
- Session domain model and statuses
- Russian date storage layout: `<tag>/DD.MM.YYYY/meeting_HH-mm-ss/`
- `meta.json` writer/reader
- Public settings load/save/validation
- Secret storage adapter via `keyring`
- SQLite session index and event log
- Commands:
  - `get_settings`
  - `save_public_settings`
  - `list_sessions`
  - `set_api_secret`
  - `get_api_secret`
  - `start_recording`
  - `stop_recording`
  - `run_pipeline`
  - `retry_pipeline`
- Pipeline stubs to call transcription and summary cloud endpoints
- React UI with recording form + settings fields + pipeline trigger
- Session list UI with manual retry action
- Tray menu with actions: Open, Start, Stop, Settings, Quit
- Dedicated Settings window (separate webview)
- Opus writer module (`ogg+opus`) with tests and valid `audio.opus` output
- Audio device discovery via `cpal` and configurable mic/system source names in Settings
- Continuous capture session (`Start -> Stop`) in background thread, spool PCM to temp raw files, stream mixed frames to Opus writer
- Auto-detect system source device by name heuristics (`loopback`, `stereo mix`, `blackhole`, `vb-cable`, etc.)
- Platform-aware detection priority (macOS prefers BlackHole/Soundflower, Windows prefers Stereo Mix/Loopback)
- System source fallback resolver: if configured device is missing, fallback to best detected loopback device (excluding selected mic)
- Automatic retry queue worker for failed pipeline jobs (SQLite-backed schedule + background polling)
- Retry scheduler/backoff logic covered by unit tests in SQLite repository layer
- Frontend runtime tests for main/tray flow and dedicated settings window flow (Vitest + Testing Library)
- Client-side settings validation in UI (URL + Opus bitrate) with RU error messages and disabled save on invalid form
- Settings window now shows RU status feedback after actions (including successful save)
- Secret save states in settings UI (`обновлён` / `не изменён` / `ошибка`) for Nexara/OpenAI keys
- Command-core module for Tauri command flow with tests for `start/stop/run/retry` validation and pipeline failure transitions
- Integration tests for command-flow persistence/retry behavior on real filesystem + SQLite (`src-tauri/tests/command_flow_integration.rs`)
- IPC invoke runtime harness tests via `tauri::test` for validation and success/failure paths (`start_recording`, `stop_recording`, `run_pipeline`, `retry_pipeline`)
- Retry-worker processing extracted to `process_retry_jobs_once` and covered for mixed outcomes + limit handling + exhausted retries
- Retry-worker tests now assert event-log outcomes (`pipeline_retry_success`, `pipeline_retry_scheduled`, `pipeline_retry_exhausted`)
- GitHub Actions matrix CI added for BigEcho on macOS + Windows (`npm test/build`, `cargo check/test`)
- GitHub Actions bundle workflow added for BigEcho (`.github/workflows/bigecho-release.yml`):
  - manual trigger (`workflow_dispatch`)
  - matrix build on macOS + Windows
  - bundle upload as workflow artifacts
  - no GitHub Release publication
  - no signing/notarization stage in CI workflow
- Modernized frontend UI with dedicated CSS theme (card layout, responsive spacing, clearer action hierarchy) for main and settings windows
- Tray-first behavior improved: closing the main window now hides it to tray instead of exiting app
- Added tests for tray-first UI/behavior:
  - frontend test checks tray-mode hint in main window
  - Rust test checks close-to-tray interception policy for main window only
- Main window header now has explicit `Свернуть в трей` action button
- Tray menu includes `Show/Hide BigEcho` quick toggle action
- App launches hidden to tray by default; env override `BIGECHO_START_HIDDEN=false` disables this behavior
- Added tests for tray launch policy and header hide action
- UI/UX update:
  - `Start` renamed to `Rec` with red recording indicator
  - `Tag` renamed to `Source` and source list expanded: `zoom`, `slack`, `telemost`, `telegram`, `browser`, `facetime`
  - `custom tag`, `topic`, `participants` are optional at record start
  - `custom tag`, `topic`, `participants` are editable after recording in `Sessions` via `Save details`
  - Recording form fields (`source/custom tag/topic/participants`) aligned in one row on desktop
  - `Sessions` now has search filters for source/custom tag/topic/participants
  - Added tray mini-window (`Recorder`) with `Rec`/`Stop`, `source`, optional `topic`
- Tray productivity update:
  - left-click on tray icon toggles recorder mini-window visibility
  - global hotkeys added: `Cmd/Ctrl+Shift+R` (Rec), `Cmd/Ctrl+Shift+S` (Stop)
  - tray tooltip now reflects recording state in real time (`BigEcho REC` / `BigEcho IDLE`)
- Session metadata UX:
  - metadata edits in `Sessions` are auto-saved with debounce (no manual save button)
- Backend additions for post-recording UX:
  - `get_session_meta` command
  - `update_session_details` command
  - Start validation relaxed to allow empty topic/participants

## Not implemented yet

- Production-grade cross-platform system loopback routing defaults (device-level nuances on real hardware)
- End-to-end tests for real Tauri command invocation (record/start/stop/pipeline) on CI
- UX polish for settings form

## Next high-priority tasks

1. Add deterministic ordering tests for retry-worker processing timestamps and event-log assertions
2. Harden cross-platform loopback defaults with environment-driven fallback config (macOS/Windows)
3. Prepare packaging profile and installer metadata for macOS + Windows distribution
4. Add onboarding docs for required virtual audio drivers per platform
5. Add optional local/distribution signing process outside GitHub Actions (if needed later)
