# Windows local mode checklist

Use this checklist for every local-whisper Windows build before reporting a runtime bug.

## 1. Start from the right build

1. Close Speakyfi from the tray.
2. Open Task Manager and end every running `speakyfi.exe` process.
3. Download the `speakyfi-windows-exe` artifact from the latest successful GitHub Actions run.
4. Start the downloaded `speakyfi.exe`.
5. Open Settings -> Diagnostics.

Expected:

- `Build Mode` is `local-whisper`.
- `Local Whisper` is `true`.
- `Build Commit` and `GitHub Run` match the artifact you downloaded.

If this does not match, the bug report is for an old or wrong build.

## 2. Verify local transcription without correction

1. Settings -> Providers: set `Provider` to `Local (whisper.cpp)`.
2. Settings -> Model: select `tiny` first and click `[ DOWNLOAD SELECTED ]`.
3. Settings -> Correction: set `Mode` to `Off`.
4. Open Notepad and place the cursor in an empty document.
5. Hold the Push-to-Talk key for 2-5 seconds, say one short phrase, then release.

Expected:

- Overlay changes from `RECORDING` to `PROCESSING`, then returns a result.
- Footer shows `INSERT OK`.
- Text appears in Notepad.
- Settings -> History contains the attempt with `INSERT OK`.

If it fails, report:

- exact footer text;
- Settings -> Diagnostics screenshot or copied values;
- Settings -> History entry for the failed attempt.

## 3. Verify Ollama correction only after transcription works

1. Confirm local transcription works with Correction `Off`.
2. Start Ollama.
3. In a terminal, run `ollama list`.
4. If the target model is missing, run `ollama pull llama3.2:1b` or pull the exact model you want.
5. Settings -> Correction: set `Mode` to `Ollama (local)`.
6. Endpoint: `http://localhost:11434`.
7. Model: exact name from `ollama list`, for example `llama3.2:1b`.
8. Click `[ CHECK ]`.
9. Repeat the Notepad recording test.

Expected:

- `[ CHECK ]` says the model is installed.
- If correction fails, raw transcription is still inserted or shown.
- History separates transcription errors from correction errors.

If Ollama reports `model not found`, either run `ollama pull <model>` or choose an installed model from `ollama list`.
