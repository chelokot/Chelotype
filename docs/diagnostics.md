Headless snapshots
- Run with `CHELOTYPE_HEADLESS=1` to spawn the shell, feed a short scripted session, and dump snapshots.
- Snapshot files go to `/tmp/chelotype_snapshots` by default or `CHELOTYPE_SNAPSHOT_DIR`.
- Each run produces `.json`, `.html`, `.txt`, and `markup.html` captures with colors, cursor, and rendered markup.
- Selection runs also produce `.render.json` with history/input markup, selected text, and the selected grid range.
- JSON snapshots include `rows`, `cols`, cursor fields, mouse mode fields, plain `text`, and per-cell text/color/style records, including zero-width combining marks and wide-cell spacer metadata.
- Headless mode runs a clean interactive `zsh -f` fixture with an empty prompt so shell line editing, arrow keys, and mouse-click cursor movement are exercised against the same kind of editable shell input path used by the product target.
- Override actions with `CHELOTYPE_HEADLESS_ACTIONS`; split actions with `|`, and use `\n`, `\r`, `\t`, `\e`, and `\\` escapes.
- Prefer `CHELOTYPE_HEADLESS_EVENTS` for input-path e2e. Supported events: `raw:<bytes>`, `text:<text>`, `key:<name>`, `resize:<cols>x<rows>`, `scroll:<lines>`, `mouse:press:<button>:<col>,<row>`, `mouse:drag:<col>,<row>`, and `mouse:release:<col>,<row>`.
- Override expected terminal text with `CHELOTYPE_HEADLESS_EXPECT`; split expected substrings with `|`.
- Override delay between actions with `CHELOTYPE_HEADLESS_STEP_MS`.
- Override selected cells with `CHELOTYPE_HEADLESS_SELECTION=row,column:row,column`.
- This runner is a binary-level snapshot/automation gate. It is not a replacement for GTK event e2e; cooked shell line-editing sequences such as Backspace and arrows should be verified through input mapping tests plus GTK interaction tests.

Live snapshots
- Set `CHELOTYPE_SNAPSHOT=1` while running the UI; one snapshot per second is written to the snapshot directory.
- Use `CHELOTYPE_SNAPSHOT_DIR` to override the dump location.
- Set `CHELOTYPE_RENDER_SNAPSHOT=1` with live snapshots to also emit `.render.json`/`.render.html` files for the active rendered frame. These include transient UI overlays such as IM preedit text at the terminal cursor, which are not part of the PTY-backed terminal grid until committed.

Usage tips
- Inspect `markup.html` for GTK markup fidelity and color mismatches.
- Compare `.txt` with `.html` to spot alignment or spacing drift.
- When debugging input lag, pair snapshots with benches in `benches/` to reproduce render cost without the UI.
