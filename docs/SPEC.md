# breakpoint: v1 spec

Status: **locked v1, 2026-11-02.** Rewritten from the decisions on the [wayfinder map](https://github.com/rawasmhd/breakpoint/issues/23); each section links the ticket that holds its detail and reasoning. Where this file and a ticket disagree, the ticket's resolution wins and this file has a bug. Companions: [GLOSSARY.md](../GLOSSARY.md) (the vocabulary, used strictly below), [RESEARCH.md](RESEARCH.md) (the evidence behind the defaults) and [ADR 0001](adr/0001-rust-for-the-desktop-app.md) (why Rust).

## 1. Pitch

**Breakpoints for humans.** A tiny Windows tray app that nudges a desk worker to stand and move, drink water and pause between focus periods. It notices when you actually step away, so a nudge you already obeyed goes away by itself.

## 2. Goals and non-goals

**Goals**
1. Useful within a minute of first launch: one wizard page, sensible defaults, nothing else to set up.
2. Invisible until needed: lives in the tray; a nudge is a small popup above the tray that never takes keyboard focus.
3. Judges by behaviour, not buttons: being away from the computer satisfies a nudge. There is no Done button.
4. Lean: one `breakpoint.exe`, no runtime, no installer, no background service, no telemetry, as few crates as possible.
5. Open source (MIT), readable, testable: the scheduling core is plain Rust with no Windows dependency.

**Non-goals in v1**
- macOS and Linux (the core stays portable; no port in this effort).
- Time tracking, task lists, Pomodoro task management, gamification, streaks.
- Cloud sync, accounts, telemetry. Nothing leaves the machine.
- An installer (MSI or setup.exe).
- Screen-blocking overlays (Hard enforcement), and any nudge other than the three below.

**Platform:** Windows 10 and 11, x86-64.

## 3. The three nudges

| Nudge | What it asks | Satisfied by | Buttons | Enforcement |
|---|---|---|---|---|
| **Stand/Move** | Stand up and move for the stand length | Being Away for the stand length, or a lock, suspend or display-off | Snooze, Skip | Soft or Firm |
| **Pause** | Step away for the pause length; ends a focus period | Being Away for the pause length, or a lock, suspend or display-off | Snooze, Skip | Soft or Firm |
| **Water** | Drink some water | Never (drinking cannot be seen); logged as shown | none | always Soft, silent |

### Profiles and defaults

Decided in [Nudge cadence defaults and profiles](https://github.com/rawasmhd/breakpoint/issues/30) and [Soft and Firm enforcement behaviour](https://github.com/rawasmhd/breakpoint/issues/31). All cadences are **active time**.

| | Gentle | **Balanced (default)** | Strict |
|---|---|---|---|
| Stand/Move every | 45 min | 30 min | 30 min |
| Stand length | 2 min | 2 min | 2 min |
| Pause every (focus period) | 90 min | 60 min | 30 min |
| Pause length | 5 min | 5 min | 5 min |
| Water every | 60 min | 60 min | 60 min |
| Stand/Move enforcement | Soft | Soft | Firm |
| Pause enforcement | Soft | **Firm** | Firm |

- Stand/Move cadence **divides** Pause cadence in every profile, so a Pause absorbs every Nth Stand/Move by design. Gentle and Balanced run Stand, Pause, Stand, Pause; Strict is a 30/5 Pomodoro where Pause absorbs every Stand/Move.
- A profile is a preset, not a stored mode: it fills the numbers and levels, every one stays editable, and numbers and levels that match no preset show as **Custom** ([Settings window scope](https://github.com/rawasmhd/breakpoint/issues/33)).
- All three nudges are on by default.

**Evidence grades** (kept attached to the defaults; see [RESEARCH.md](RESEARCH.md)): breaking up sitting, moderate, but the number 30 is weak (no guideline fixes an interval); stand length 2 min, moderate; pause length 5 min, moderate (micro-break meta-analysis); Pause cadence 60 and 30, weak (Pomodoro and ultradian cycles are secondary-source doctrine), 90 moderate; Water every 60 min, **habit cue**: there is no evidence for any drinking interval, the number is EFSA's daily total spread over a desk day.

### Nudge text

| Nudge | Title | Line |
|---|---|---|
| Stand/Move | Stand up and move | Stand, roll your shoulders, look at something far away. |
| Pause | Pause | Step away from the screen for 5 minutes. (the pause length) |
| Water (riding along) | | Drink some water. |
| Water (alone) | Drink some water | |

The wording is copy, free to polish without a product decision; what each nudge asks is not. The Stand/Move line carries the posture and eyes cue; eyes and posture are not separate nudges.

## 4. Presence and active time

From [Activity and presence detection on Windows](https://github.com/rawasmhd/breakpoint/issues/25) ([findings](https://github.com/rawasmhd/breakpoint/blob/research/presence-detection/docs/research/presence-detection.md)).

One **Presence** state machine: `Active`, `AtDesk`, `Away { since, reason }`, `Unknown`.

- **Poll every 5 s:** `GetLastInputInfo`. Input within the last 90 s is `Active`.
- **No input for 90 s or more without a veto** is `Away` with reason *no input*, and `since` is **the time of the last input**, not the moment it was detected. Every length check (stand length, pause length, the resets in §5.2, the popup countdown) measures from `since`.
- **Vetoes:** when the shell reports full-screen, busy or presentation mode (`SHQueryUserNotificationState`), an app holds the display on (`ES_DISPLAY_REQUIRED`), or media is playing (Global System Media Transport Controls), 90 s or more without input is `AtDesk` instead: present but not typing. Vetoes are checked only once input has stopped for 10 s. If the media-controls call fails once, breakpoint runs without that veto for the rest of the session.
- **Events win over the poll:**
  - Session lock, console or remote disconnect (including fast user switching) and suspend make the user `Away` at once; they count as a **lock** (§5.2, §5.5), logged `lock` or `suspend`.
  - Display off (`GUID_SESSION_DISPLAY_STATUS`) makes the user `Away` at once and counts like a lock, logged `display_off`.
  - Windows' user-inactive verdict (`GUID_SESSION_USER_PRESENCE`) makes the user `Away` with `since` at the last input; it satisfies only after the length, like no input, logged `away`.
  - Unlock, connect, resume from suspend, display on and user-present make the user `Active`. An automatic resume alone is `Unknown` until input arrives.
- The 90 s threshold, the 5 s poll and the vetoes are **hard-coded**, not settings.
- No camera, no presence sensor, no keyboard hooks, no inspection of other processes.

**Active time** is time spent `Active` or `AtDesk`. Every cadence, the snooze and the Firm repeat count active time only; time `Away` or `Unknown`, and all quiet time, does not advance them. Active time is not backdated: the up to 90 s of silence before `Away` is declared counts as active.

**Known blind spots, accepted:** standing at a sit-stand desk while typing (looks Active; the user presses Skip), hardware or software mouse jigglers, a windowed video that neither sets display-required nor reports to the media controls (can look Away), long reading without input (can look Away).

## 5. An occurrence, start to end

Decided in [How a nudge is satisfied](https://github.com/rawasmhd/breakpoint/issues/28), [Define Stop Working](https://github.com/rawasmhd/breakpoint/issues/24) and [Soft and Firm enforcement behaviour](https://github.com/rawasmhd/breakpoint/issues/31).

### 5.1 Firing

- Each nudge has a cadence clock in active time. When it reaches the cadence, an **occurrence fires** and the clock restarts from zero.
- A nudge never has two pending occurrences: when the next one falls due while the last is still pending, the old one **expires** (cause `next`) and the new one fires.
- No warning is shown before any nudge.

### 5.2 Presence resets clocks

- Any `Away` lasting at least the **stand length**, or any lock, restarts the Stand/Move clock from zero, even with nothing pending.
- Any `Away` lasting at least the **pause length** restarts the Pause clock: the focus period starts again from zero when the user comes back (lunch, a meeting).
- A **satisfied** occurrence restarts its own nudge's clock, so a Pause satisfied by a short lock still starts a fresh focus period when the user comes back. (Clocks do not run while the user is away, so "at the return" and "at satisfaction" are the same moment.)
- A Pause the user returns from early stays pending (§5.5); its clock is not restarted.
- The Water clock is never reset by presence.

### 5.3 Combined nudge

- When the first of Stand/Move and Pause falls due and the other is due **within the next 2 active minutes**, both fire then as one **combined nudge**, and both clocks restart.
- Whenever a Stand/Move and a Pause are **pending at the same time**, for whatever reason (they drifted out of phase after an absence, or a custom cadence that does not divide), they are one combined nudge from then on: a Stand/Move that fires while a Pause is pending joins it (its `fired` line carries `into`), and a Pause that fires while a Stand/Move is pending takes it in (no extra log line).
- In a combined nudge the **Pause absorbs the Stand/Move**: one popup, Pause's title and countdown, one Snooze and one Skip that act on both occurrences. If either was already snoozed, the combined nudge counts as snoozed (Skip only).
- Each absorbed occurrence is still satisfied on its own: Away for the stand length satisfies the Stand/Move while the Pause stays pending until the pause length.
- A combined nudge is **Firm if either part is Firm**.
- The 2-minute window is hard-coded.

### 5.4 Water rides along

- Water's clock makes it **due** every 60 active minutes; it does not prompt by itself while Stand/Move or Pause is on.
- A due Water adds the line "Drink some water." to the popup of the **next newly fired** Stand/Move or Pause occurrence. That is its *shown* event, and the Water clock restarts then. It never joins a popup already on screen, a Firm repeat or a post-snooze re-prompt. A line that rode on a popup's first show stays on that occurrence's re-prompt and Firm repeats, so they are identical (§5.7), and is logged as `shown` once. While it waits, a second due does nothing.
- Quiet start drops a waiting Water (every clock restarts when quiet ends anyway).
- Whenever Water is due and Stand/Move and Pause are both off, Water fires alone at once as a small silent popup with no buttons that hides itself after about 7 seconds.
- Water never changes a popup's enforcement level and never adds a button.

### 5.5 Ending: satisfied or skipped

A Stand/Move or Pause occurrence is **pending** from firing until it ends in exactly one way.

**Satisfied** (`by` in the log):
- `away`: the user is `Away` continuously for the stand length (Stand/Move) or the pause length (Pause), measured from the last input. Coming back sooner leaves it pending; short absences do not add up.
- `lock`, `suspend`, `display_off`: the event satisfies it at once.
- Detection keeps running while an occurrence is snoozed or its popup is closed.

**Skipped:**
- `user`: the user pressed **Skip** (popup or tray). Only that occurrence ends; the next comes as usual.
- **Expired**, recorded with its cause:
  - `next`: the next occurrence of the same nudge fell due.
  - `quiet`: quiet time started (§7), including a settings change that makes now quiet.
  - `settings`: the user turned that nudge off in Settings.
  - `exit`: breakpoint quit (tray Quit, or Windows ending the session).

There is no Done button. Closing a popup is not a press: the occurrence stays pending.

### 5.6 Snooze

- **Snooze** pushes a pending occurrence **5 active minutes** later. Fixed, not a setting.
- Allowed **once** per occurrence. When the snooze ends, the occurrence is prompted again once with **Skip only**.
- A snoozed occurrence is still pending: walking away for the length satisfies it and cancels the re-prompt.

### 5.7 Enforcement: Soft and Firm

- **Soft:** one popup per occurrence. It stays on screen until the user snoozes, skips or closes it. Nothing is shown again before the occurrence ends, except the one re-prompt after a snooze.
- **Firm:** everything Soft does, plus the popup **comes back every 5 active minutes** while the occurrence is pending, until it is satisfied, skipped or expired. The repeat is identical (same text, same size, same sound), replaces the one on screen rather than stacking, and has no cap. After the one snooze, repeats carry Skip only. The 5 minutes are fixed.
- **Sound:** every Stand/Move and Pause popup (first show, re-prompt, Firm repeat) plays the Windows default notification sound. Water is silent.
- A pending occurrence keeps the level and the length it fired with, even if Settings change them.

## 6. The popup

Decided in [Notification channel: toast vs popup](https://github.com/rawasmhd/breakpoint/issues/34) (throwaway prototypes on [`prototype/notification-channel`](https://github.com/rawasmhd/breakpoint/tree/prototype/notification-channel)). Windows toasts are **not used** in v1: Do not disturb swallows them silently and no API reveals that.

- breakpoint's own small raw-Win32 window, bottom-right of the primary monitor's work area, just above the tray. Always on top.
- Shown **without taking focus** (`WS_EX_NOACTIVATE`, `SW_SHOWNOACTIVATE`, `MA_NOACTIVATE`), so typing never lands on it.
- Content: title, line, a riding Water line if any, a **live away countdown** of the stand or pause length measured from the last input ("Away 1:59 more and it counts"; restarts on any input), and the buttons **Snooze 5 min** and **Skip** (Skip only after the snooze). While a veto applies (§4) the countdown holds at the full length and does not run. The countdown is a display of Presence; satisfaction is decided by Presence (§5.5), never by the countdown alone.
- **X** hides it; the occurrence stays pending and is still reachable from the tray. Snooze hides it until the re-prompt. **Any ending** (satisfied, skipped, any expiry) closes it; an expiry with cause `next` is replaced by the new occurrence's popup.
- At most one popup on screen. A Firm repeat brings the same window back unchanged.
- It **ignores Windows Do not disturb** by design; only breakpoint's quiet rules silence it. In v1 this means a popup can appear over a presentation or screen share (see §15).
- Standard light Win32 look (comctl32 v6 controls, system message font), PerMonitorV2 DPI; the popup handles `WM_DPICHANGED` itself.

## 7. Quiet, the work week, Off and Overtime

Decided in [Quiet rules and the off switch](https://github.com/rawasmhd/breakpoint/issues/29).

- Nudges fire only inside **work hours** on a **work day**. Defaults: **Sunday to Thursday, 08:00 to 17:00** (weekend **Friday and Saturday**). Hard-coded defaults, not locale-aware; all editable in Settings.
- End must be after start; overnight hours are not supported in v1.
- Every other moment is **quiet**: outside hours, on the weekend, or **Off**.
- **Starting quiet** expires every pending occurrence (cause `quiet`). **Leaving quiet** restarts every clock from zero: a fresh focus period. Clocks never carry across quiet time.
- **Off:** during work hours the top tray item reads **Turn off until next work day**. Off ends by itself at the next work day's start time, or earlier from the same item.
- **Overtime:** outside hours or on the weekend the top item reads **Turn on until midnight**. Nudges then run normally until 00:00. The same item ends it early.
- Off and Overtime are saved in `config.json` as the moment they end, so a restart or reboot keeps them. At most one is in force; starting one clears the other.

The top tray item, tooltip and icon in every case:

| Now | Item reads | Pressing it | Tooltip | Icon |
|---|---|---|---|---|
| In hours, nudging | Turn off until next work day | starts Off | `breakpoint` | normal |
| In hours, Off | Turn back on | ends Off (clocks restart) | `breakpoint: off until Sunday 08:00` | greyed |
| Outside hours, quiet | Turn on until midnight | starts Overtime (clocks restart) | `breakpoint: quiet until Sunday 08:00` | greyed |
| Outside hours, Off still in force | Turn on until midnight | ends Off, starts Overtime | `breakpoint: off until Sunday 08:00` | greyed |
| Overtime, outside hours | Turn off now | ends Overtime | `breakpoint: on until midnight` | normal |
| Overtime, in hours (started before work began) | Turn off until next work day | ends Overtime, starts Off | `breakpoint: on until midnight` | normal |

Overtime that runs into work hours keeps going until midnight; work hours starting under it is not a quiet edge, so no clock restarts.
- Nothing else quiets breakpoint automatically in v1: no full-screen, presenting, meetings, Focus Assist or app rules; no holiday calendar; no hotkey.

## 8. Tray

Uses `tray-icon` on breakpoint's own Win32 message loop. Left or right click opens the menu; there is no click-to-toggle.

**Icon and tooltip:** the normal icon while nudges can fire, a greyed one while quiet; tooltip per the table in §7.

**Menu, top to bottom:**
1. The off switch (labels in §7).
2. Separator.
3. Only while an occurrence is pending: a disabled line naming it (e.g. *Pause pending*), then **Snooze 5 min** (hidden once used) and **Skip**. These act exactly like the popup's buttons, on the whole combined nudge if it is one.
4. **Today count**, a disabled line: `Today: 7 taken, 2 skipped` (§10.3).
5. Separator.
6. **Settings…**
7. **Quit**

## 9. First-run wizard and settings window

Both are raw Win32 `DIALOGEX` templates (from [UI technology for Rust windows](https://github.com/rawasmhd/breakpoint/issues/26)), caption *breakpoint*, app icon, comctl32 v6 and PerMonitorV2 from the manifest.

### 9.1 First-run wizard

Decided in [First-run wizard questions and public defaults](https://github.com/rawasmhd/breakpoint/issues/32). Shown only when `config.json` does not exist.

One page that **asks two things**:
1. **Profile:** three radio buttons, **Balanced** preselected, each with one plain line of its numbers, e.g. *Balanced: stand every 30 min, pause every 60 (Pause insists). Water rides along hourly.*
2. **Start with Windows:** checkbox, **on**.

And **shows two things**:
3. A read-only line *Nudges Sunday to Thursday, 08:00 to 17:00* with a **Change…** button.
4. *breakpoint lives in the tray, under ^ if you don't see it.*

- **Start** (default button) writes the config and starts nudging.
- Closing it (X or Esc) is the same as Start: accepting what the page shows. The wizard never appears again; there is no re-run (deleting `config.json` brings it back as a side effect).
- **Change…** closes the wizard and opens the settings window at the same position, pre-filled with the wizard's picks and defaults for the rest, focus on the first work-day checkbox. **Save** there writes the config and finishes first run; **Cancel**, Esc or X finishes first run as closing the wizard would (wizard picks plus defaults written, settings edits discarded).
- Enforcement is never asked; the profile sets it.

### 9.2 Settings window

Decided in [Settings window scope](https://github.com/rawasmhd/breakpoint/issues/33). One fixed-size page, no tabs, titled *breakpoint settings*, opened from **Settings…**. Only one instance; Settings… on an open window brings it to the front. Nudges keep firing while it is open.

Top to bottom:
- **Work week:** seven *Work days* checkboxes, Sunday first (Friday and Saturday unchecked by default); work hours **from** and **to** as stock date-time pickers in 24-hour `HH:mm`.
- **Profile:** drop-down Gentle / Balanced / Strict; shows **Custom** when the cadences, lengths and enforcement levels match no preset exactly (display only, not choosable). Picking a profile fills cadences, lengths and levels in the form, leaves the on/off boxes alone, and saves nothing until Save. Editing any of those fields flips the drop-down to Custom live. There is no separate restore-defaults button.
- **Nudges**, one row each (edit box with an up-down control, whole minutes):
  - *Stand/Move* [on] every [30] min, stand length [2] min, [Soft / Firm]
  - *Pause* [on] every [60] min, pause length [5] min, [Soft / Firm]
  - *Water* [on] every [60] min, static note *Always gentle; rides along with the next nudge.*
- **Start with Windows** checkbox.
- **Save** (default, Enter) and **Cancel** (Esc, and X). Cancel discards without asking.

**Ranges:** Stand/Move cadence 10 to 240, Pause cadence 20 to 240, Water cadence 10 to 240, stand length 2 to 5, pause length 2 to 15. A Stand/Move cadence that does not divide the Pause cadence is allowed with no warning. All three nudges may be off.

**Save** validates first: an out-of-range value, an end time not after the start, or no work day checked shows one message box naming the field, focuses it and saves nothing. A valid Save writes `config.json` and applies at once:
- A nudge whose on/off, cadence or length changed restarts its clock from zero. Its pending occurrence stays pending and expires at the new next due. Turning a nudge **off** expires its pending occurrence (cause `settings`).
- An enforcement-only change applies from that nudge's next occurrence.
- Work days or hours are re-evaluated with the quiet rules at once (now quiet: pending occurrences expire with cause `quiet`; quiet just ended: every clock restarts). An Off or Overtime in force keeps its end moment.
- Start with Windows writes or deletes the Run value.

**Not settings anywhere** (hard-coded): snooze 5 active min, Firm repeat 5 active min, Away threshold 90 s, presence poll 5 s, combined-nudge window 2 min. The log has no control in any UI.

## 10. Storage

Everything lives in **`%APPDATA%\breakpoint\`**, created on first write. Decided in [Event log schema and the Today tray item](https://github.com/rawasmhd/breakpoint/issues/36) and [Settings window scope](https://github.com/rawasmhd/breakpoint/issues/33).

### 10.1 `config.json`

Holds exactly the settings window's fields plus the Off / Overtime end moment. The profile is not stored; Start with Windows is not stored (the Run key is its truth).

```json
{
  "workDays": ["sun", "mon", "tue", "wed", "thu"],
  "workHours": { "start": "08:00", "end": "17:00" },
  "stand": { "on": true, "everyMinutes": 30, "lengthMinutes": 2, "enforcement": "soft" },
  "pause": { "on": true, "everyMinutes": 60, "lengthMinutes": 5, "enforcement": "firm" },
  "water": { "on": true, "everyMinutes": 60 },
  "offUntil": null,
  "overtimeUntil": null
}
```

- `offUntil` and `overtimeUntil` are local times with offset (RFC 3339) or `null`; at most one is set.
- Read **once at start-up**, not watched. Save rewrites it whole, so hand edits made while breakpoint runs are lost on the next Save. Hand edits are possible but unsupported.
- **Unparseable file:** left untouched; breakpoint runs on the defaults and says so **once per start** with a **notice**: a small window in the popup's place, no buttons, staying until closed with X: *breakpoint couldn't read config.json and is using the defaults. Saving from Settings will replace the file.* A nudge popup that needs the spot replaces the notice.
- **A value that is missing, out of range or malformed** (an unknown day name, an empty `workDays`, end not after start): only that value falls back to its default, with the same single notice; the rest applies. If both `offUntil` and `overtimeUntil` are set, both are ignored. An end moment already in the past is ignored silently. The file is rewritten only when the user saves from Settings.
- An unparseable file does not trigger the wizard; only a missing one does.

### 10.2 Event log

- **JSON Lines, one file per calendar month:** `%APPDATA%\breakpoint\log\YYYY-MM.jsonl`. UTF-8, `\n` line endings.
- **Append one line per event, opening and closing the file each time.** Best-effort: a failed write is dropped silently; nudges never depend on the log. Readers skip malformed lines and ignore unknown fields.
- **Kept forever.** No pruning, no in-app clear, no setting. The user may delete any file or the folder by hand; breakpoint recreates it on the next write.
- **Local only.** No window titles, app names or input details are ever recorded.

Only five events are logged: `fired`, `snoozed`, `satisfied`, `skipped` (Stand/Move and Pause) and `shown` (Water). Not logged: Firm repeats, the post-snooze re-prompt, short absences, quiet/Off/Overtime transitions, app start, settings changes. No schema version field; later versions only add fields.

| Field | Type | On | Meaning |
|---|---|---|---|
| `t` | string | every line | Local time with offset, RFC 3339, whole seconds |
| `ev` | string | every line | `fired` \| `snoozed` \| `satisfied` \| `skipped` \| `shown` |
| `nudge` | string | every line | `stand` \| `pause` \| `water` |
| `occ` | integer | every line | Occurrence id, increasing across the whole log and never reset; at start-up it continues from the highest `occ` in the newest file (1 if none). A Water `shown` gets its own. |
| `by` | string | `satisfied`, `skipped` | On `satisfied`: `away` \| `lock` \| `suspend` \| `display_off`. On `skipped`: `user` \| `next` \| `quiet` \| `settings` \| `exit` (everything but `user` is an expiry). |
| `into` | integer | `fired` of an absorbed Stand/Move | The `occ` of the Pause that absorbed it |

Fields are written in that order; absent fields are omitted, never `null`. In a combined nudge each occurrence gets its own lines, and one Snooze or Skip press writes one line per occurrence it acts on. A crash writes nothing for the pending occurrence; readers tolerate a `fired` with no end.

```jsonl
{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"pause","occ":42}
{"t":"2026-11-02T09:00:00+03:00","ev":"fired","nudge":"stand","occ":43,"into":42}
{"t":"2026-11-02T09:00:00+03:00","ev":"shown","nudge":"water","occ":44}
{"t":"2026-11-02T09:00:40+03:00","ev":"snoozed","nudge":"pause","occ":42}
{"t":"2026-11-02T09:00:40+03:00","ev":"snoozed","nudge":"stand","occ":43}
{"t":"2026-11-02T09:07:02+03:00","ev":"satisfied","nudge":"pause","occ":42,"by":"lock"}
{"t":"2026-11-02T09:07:02+03:00","ev":"satisfied","nudge":"stand","occ":43,"by":"lock"}
{"t":"2026-11-02T17:00:00+03:00","ev":"skipped","nudge":"pause","occ":58,"by":"quiet"}
```

### 10.3 Today count

- One total, exactly `Today: 7 taken, 2 skipped` (zero state `Today: 0 taken, 0 skipped`; no plural changes).
- **Taken** = Stand/Move and Pause occurrences that ended `satisfied`; **skipped** = those that ended `skipped`, any `by`. Water is excluded; snoozes change nothing. An absorbed Stand/Move counts on its own, so a taken combined nudge adds 2.
- The day ends at **local midnight**; an occurrence counts toward the day it **fired**.
- Kept in memory; at start-up rebuilt from the current month's file (end events whose `fired` line is dated today). An occurrence left open by a crash is ignored.

## 11. Start with Windows, single instance, quitting

- **Start with Windows** is the per-user Run key value (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run\breakpoint`). On by default from the wizard. On every start, if the value exists, breakpoint rewrites it with its current exe path, so moving the exe does not break autostart.
- **Single instance:** a named mutex. A second launch tells the running instance to open its Settings window (or bring the wizard to the front during first run) and exits. This is how someone whose tray icon is hidden in the overflow finds breakpoint.
- **Quit** (tray) and Windows ending the session expire any pending occurrence (cause `exit`) before exiting.

## 12. Technical design

### 12.1 Stack

**Rust, stable, MSVC toolchain** (`x86_64-pc-windows-msvc`), locked in [ADR 0001](adr/0001-rust-for-the-desktop-app.md). The dev machine and CI build the same target ([Make the windows crate build locally](https://github.com/rawasmhd/breakpoint/issues/35)). Do not pin old `windows` crates to support the GNU toolchain.

| Concern | Crate | Notes |
|---|---|---|
| Win32 and WinRT calls, dialogs, popup | `windows` | Presence APIs, power and session notifications, `SHQueryUserNotificationState`, media controls (WinRT), Run key, named mutex, `PlaySound`, `DIALOGEX` dialogs, the popup window |
| Tray icon and menu | `tray-icon` (with its `muda` menus) | Needs only a Win32 message loop on the thread; no `winit` |
| Config and log | `serde` + `serde_json` | No file watcher, no date-time crate: the platform layer supplies local time and UTC offset |
| Exe resources | `winresource` (build dependency) | Icon, version info, manifest (comctl32 v6 + PerMonitorV2), `DIALOGEX` templates via `append_rc_content` |
| Tests | built-in `cargo test` | Core tested with an injected clock and presence feed |

No other runtime crates without a decision. Notably not used: toast crates (`tauri-winrt-notification`), GUI toolkits (egui, Slint, iced, WebView2/wry, native-windows-gui), `jiff`/`chrono`.

### 12.2 Architecture

```
src/
  main.rs            single-instance guard, message loop (with IsDialogMessage), wiring
  core/              no `windows` dependency; compiles and tests on any OS
    scheduler.rs     clocks in active time, firing, combining, Water riding, snooze, Firm repeats, expiry
    presence.rs      Presence state machine fed by polls and events (inputs supplied by platform)
    quiet.rs         work week, Off, Overtime, quiet edges
    config.rs        model, defaults, profiles, validation, JSON load/save
    log.rs           event records, JSONL formatting, Today count rebuild
  platform/          Windows only
    tray.rs          tray-icon menu and icon states
    presence.rs      GetLastInputInfo, vetoes, session and power notifications
    popup.rs         the nudge popup window
    dialogs.rs       wizard and settings DIALOGEX procedures
    startup.rs       Run key
    clock.rs         local time and UTC offset
tests/
  scheduler.rs       scenario tests with a fake clock and presence feed
```

**The one rule:** `core/` never depends on the `windows` crate. The scheduler is a pure function of config, the current time, presence and user actions, producing the popups to show and the log lines to write.

### 12.3 Build

- Release profile: `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, `strip = true`; `#![windows_subsystem = "windows"]` (no console).
- Size reference (prototypes, this profile): bare `windows` exe 110 KB; with the popup 153 KB.
- **Dev aid, debug builds only:** the environment variable `BREAKPOINT_MINUTE_SECS` (a positive number, fractions allowed) makes one minute that many seconds, for every duration given in minutes: cadences, stand and pause lengths, snooze, the Firm repeat and the combined-nudge window. The durations given in seconds (the 90 s Away threshold, the 5 s poll, the 10 s veto delay, Water alone's 7 s) stay real. Release builds ignore it.
- Presence registration must retry `WTSRegisterSessionNotification` on early-logon failure, re-arm idle measurements after any resume, and fall back to a hidden top-level window if suspend broadcasts do not reach a message-only window (see the presence findings, "What this design will get wrong").

## 13. Distribution

- GitHub Releases: `breakpoint.exe` plus a SHA-256 checksum, built by GitHub Actions (MSVC) on every `v*` tag.
- No installer. winget (and Scoop) manifests later.
- Unsigned in v1: the README documents SmartScreen's *More info, Run anyway*.
- **Uninstall** is documented in the README: uncheck Start with Windows (or delete the Run value), quit, delete the exe and `%APPDATA%\breakpoint\`.
- License: MIT.

## 14. Glossary of record

Terms used above are defined in [GLOSSARY.md](../GLOSSARY.md): nudge, occurrence, pending, satisfied, skipped, expired, skip, snooze, away, stand length, pause length, cadence, focus period, Pause, combined nudge, active time, work day, weekend, quiet, Off, Overtime, profile, enforcement level, popup, event log, Today count.

## 15. Known v1 limitations

- The popup shows over presentations, screen shares and full-screen apps when they fall inside work hours: it ignores Do not disturb, and automatic quiet contexts are not in v1.
- Standing while typing at a sit-stand desk is not detected; the user presses Skip.
- Water cannot be acknowledged; it is a heads-up only.
- Light theme only for the popup and dialogs.
- Rawas's week is the hard-coded default for everyone; other users change it in Settings.

## 16. Filled in during the rewrite

The map left these small gaps; the rewrite closed them with the leanest option, under Rawas's delegation of 2026-11-02 ("Choose the recommended options from your end"). Each can be overturned without touching anything else.

1. **Presence timing:** `Active` is input within 90 s; `AtDesk` (renamed from the research's IdleAtDesk) is a veto beyond that; `Away.since` is the last input; active time counts `Active` and `AtDesk` and is not backdated (§4).
2. **Disconnect, fast user switching and display-off count as a lock**; Windows' user-inactive verdict counts as no input (§4).
3. **The presence sensor is dropped** (the research used it opportunistically when present): few machines have one and it adds a WinRT sensors dependency. Back to the fog (§17).
4. **The focus period's "long absence"** is `Away` for at least the pause length, and any satisfied occurrence restarts its own clock (§5.2).
5. **Early return from a Pause** leaves it pending and does not restart the focus period: How a nudge is satisfied (pending until satisfied) overrode the earlier "returning early still restarts" from Define Stop Working, and the event log ticket dropped "cut short" (§5.2).
6. **Combined nudge:** fires when the first of the two falls due; a Stand/Move and a Pause pending at the same time always merge, Pause absorbing (§5.3).
7. **Water** joins only a newly fired popup, its clock restarts when shown, quiet start drops a waiting Water, and with both other nudges off it fires alone at once (§5.4).
8. **A pending occurrence keeps the length it fired with** after a settings change, as it keeps its level (§5.7).
9. **The countdown holds during a veto**; any ending closes the popup (§6).
10. **The off switch in every state** (§7 table), menu order and icon (§8).
11. **The `config.json` shape** and its fallbacks (§10.1).
12. **The bad-config notice** sits in the popup's place, no buttons, until closed (§10.1).
13. **Single instance**: a second launch opens Settings in the running instance (§11).
14. **The wizard's button** is named Start (§9.1).
15. **Light theme only** in v1; dark mode moves to the fog (§15, §17).
16. **Nudge copy** (§3).
17. **Suspend and display-off satisfy a Pause** as they do a Stand/Move (the Pause ticket named only lock; the event log's `by` values already allowed both) (§3, §5.5).

## 17. Later

Fog left on the map, revisited after living with v1:
- End-of-day stop (a nudge at the end of the work day or after a daily active-time budget).
- More nudges (eyes 20-20-20, lunch, posture, wrists, breathing, daylight, start-of-day) and user-defined nudges.
- Hard enforcement (a screen overlay).
- Automatic quiet contexts: full screen, presenting, Focus Assist, named apps, meetings. Pressing, since the popup ignores Do not disturb.
- Auto-detected work day; overnight work hours; a holiday calendar or "off until a date".
- A Water acknowledgement; counting standing while typing; using a presence sensor where the machine has one.
- Looking back at past days (a weekly view or export).
- Sounds per nudge; dark mode.
- Code signing.

## 18. Decision index

| Ticket | Sections |
|---|---|
| [Define Stop Working (resolved: it is Pause, a work cycle)](https://github.com/rawasmhd/breakpoint/issues/24) | 3, 5 |
| [Activity and presence detection on Windows](https://github.com/rawasmhd/breakpoint/issues/25) | 4, 12.3 |
| [UI technology for Rust windows](https://github.com/rawasmhd/breakpoint/issues/26) | 9, 12.1 |
| [Toast capabilities for unpackaged Rust apps](https://github.com/rawasmhd/breakpoint/issues/27) | 6 (why not toasts) |
| [How a nudge is satisfied](https://github.com/rawasmhd/breakpoint/issues/28) | 3, 5 |
| [Quiet rules and the off switch](https://github.com/rawasmhd/breakpoint/issues/29) | 7, 8 |
| [Nudge cadence defaults and profiles](https://github.com/rawasmhd/breakpoint/issues/30) | 3, 5.4 |
| [Soft and Firm enforcement behaviour](https://github.com/rawasmhd/breakpoint/issues/31) | 3, 5.7 |
| [First-run wizard questions and public defaults](https://github.com/rawasmhd/breakpoint/issues/32) | 9.1 |
| [Settings window scope](https://github.com/rawasmhd/breakpoint/issues/33) | 9.2, 10.1 |
| [Notification channel: toast vs popup](https://github.com/rawasmhd/breakpoint/issues/34) | 6 |
| [Make the windows crate build locally (MSVC toolchain)](https://github.com/rawasmhd/breakpoint/issues/35) | 12.1 |
| [Event log schema and the Today tray item](https://github.com/rawasmhd/breakpoint/issues/36) | 10.2, 10.3 |
