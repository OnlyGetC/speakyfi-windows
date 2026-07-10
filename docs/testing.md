# Speakyfi Windows testing workflow

This project uses an acceptance checklist as the quality contract for each Windows build.

## Flow

1. Update `quality/acceptance-checklist.json` when the product definition changes.
2. GitHub Actions runs compile checks, Rust tests, the Tauri Windows build, and checklist report generation.
3. CI uploads `CHECKLIST_REPORT.md` together with the Windows artifacts.
4. Ivan manually checks only the items marked `MANUAL`.
5. Bugs found during manual checks should reference the checklist item id.

## Local report

Run from the repository root:

```bash
node scripts/generate-checklist-report.mjs
```

The script writes `CHECKLIST_REPORT.md`.

## Manual section

Manual checks currently cover the parts that need a real Windows desktop session:

- installer launch behavior;
- tray/settings UI;
- real microphone recording;
- PTT press/release feel;
- transcription with a real provider key;
- text insertion into a foreground app.
