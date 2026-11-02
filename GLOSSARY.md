# breakpoint

Vocabulary for a Windows tray app that nudges a desk worker to stand, drink water and pause between focus periods. One context; this file is the whole glossary.

## Language

**Nudge**:
A recurring prompt to do one healthy thing: stand and move, drink water, pause. The unit of everything breakpoint does.
_Avoid_: reminder, alert, notification (that is the OS mechanism, not the thing)

**Occurrence**:
One firing of a nudge. A nudge has many occurrences a day; each is pending from the moment it fires until it ends, satisfied or skipped. A nudge never has two pending occurrences at once.
_Avoid_: instance, event, alarm

**Pending**:
The state of an occurrence that has fired and has not yet been satisfied or skipped. Snoozing keeps it pending.
_Avoid_: open, active, outstanding

**Satisfied**:
How an occurrence ends well: the user did the thing. For Stand/Move and Pause only being away shows it; a Water occurrence is never satisfied because drinking cannot be seen.
_Avoid_: done, completed, acknowledged, dismissed

**Skipped**:
How an occurrence ends without the thing being done: the user pressed Skip, or the occurrence expired.
_Avoid_: missed, ignored, dismissed, failed

**Expired**:
The way a pending occurrence is skipped without the user pressing Skip: the next occurrence of the same nudge falls due, quiet starts, the user turns that nudge off, or breakpoint quits. The log records which it was.
_Avoid_: timed out, lapsed, stale

**Skip**:
Ending a pending occurrence as skipped on purpose. Affects only that occurrence; the nudge stays on and its next occurrence comes as usual.
_Avoid_: dismiss, ignore, cancel, mute

**Away**:
The presence state in which the user is not at the computer: no input for a while, with nothing on screen that explains the silence, or the session locked or the machine asleep. Any input ends it.
_Avoid_: idle, inactive, AFK, absent

**Stand length**:
How long the user must stay away for a Stand/Move occurrence to count as satisfied. Being away that long also resets the Stand/Move cadence when nothing is pending.
_Avoid_: stand duration, break length, minimum break

**Cadence**:
How often a nudge fires, measured in active time.
_Avoid_: interval, frequency, schedule

**Focus period**:
A run of active time between two Pauses. Thirty to ninety minutes long, depending on the profile; restarts from zero when the user comes back from a Pause or any long absence.
_Avoid_: pomodoro, session, sprint, work block

**Pause**:
The nudge that ends a focus period: step away from the desk for a few minutes, then come back. Not the end of the work day; that nudge does not exist in v1.
_Avoid_: stop working, break, long break, rest

**Pause length**:
How long the user must stay away for a Pause occurrence to count as satisfied. Five minutes in every profile.
_Avoid_: break length, rest duration

**Combined nudge**:
One prompt shown when several nudges fall due together. A Pause absorbs the Stand/Move it coincides with, so acting on the prompt acts on both; a due Water adds its line to whatever nudge is shown next and never prompts on its own while another nudge is on.
_Avoid_: merged notification, stacked reminder, batch

**Popup**:
breakpoint's own small window above the tray that shows a nudge, with its Snooze and Skip buttons. It appears on top without taking focus, comes back unchanged for a Firm repeat, and ignores Windows Do not disturb; only breakpoint's own quiet rules silence it. The only way a nudge shows itself in v1.
_Avoid_: toast, notification, dialog, alert

**Active time**:
Time the user is actually at the computer. Idle, locked and asleep periods do not count, so a cadence of 30 minutes means 30 minutes of being at the keyboard.
_Avoid_: uptime, wall-clock time, elapsed time

**Work day**:
A day on which breakpoint is on, together with the hours the user works on it. No nudge marks the end of it. For Rawas: Sunday to Thursday, 08:00 to 17:00.
_Avoid_: schedule, shift, business hours

**Weekend**:
The days the user does not work and breakpoint stays quiet. Friday and Saturday by default; not necessarily Saturday and Sunday.
_Avoid_: days off, holidays

**Quiet**:
The state in which no nudge fires: outside work hours, on the weekend, or turned off. Starting quiet expires any pending occurrence; leaving it starts every cadence from zero.
_Avoid_: silent, muted, paused, do not disturb

**Off**:
Quiet by the user's choice on a work day, until the next work day starts or the user turns breakpoint back on. Survives a restart.
_Avoid_: disabled, paused, snoozed

**Overtime**:
Nudges turned on by the user outside work hours or on the weekend, until midnight.
_Avoid_: override, extra hours, manual mode

**Profile**:
A named preset of cadence numbers and enforcement levels: Gentle, Balanced or Strict. Picking one pre-fills them; every number stays individually editable afterwards. The profile is not stored but read off the numbers: when they match no preset, the settings window shows **Custom**.
_Avoid_: mode, preset, level

**Enforcement level**:
How insistent a nudge is, chosen per nudge for Stand/Move and Pause; Water is always Soft. **Soft** prompts once and the prompt waits until the user acts on it or closes it. **Firm** does the same and then prompts again, unchanged, every few minutes of active time until the occurrence is satisfied, skipped or expired. A combined nudge is Firm if anything it absorbs is Firm. **Hard** (not in v1) blocks the screen for the break.
_Avoid_: strictness, severity, priority

**Snooze**:
Pushing a pending occurrence a few minutes into the future without ending it. It is still pending while snoozed, so being away can still satisfy it.
_Avoid_: postpone, delay, remind later

**Event log**:
The local record of what happened to each occurrence: fired, snoozed, satisfied or skipped, and shown for Water. One line per event, one file per month, kept on this computer only and never sent anywhere. The Today count is built from it.
_Avoid_: history, stats, telemetry, journal, activity log

**Today count**:
The tray line that shows how many Stand/Move and Pause occurrences fired today were taken (satisfied) and how many were skipped, e.g. "Today: 7 taken, 2 skipped". Water is not counted, and the count starts again at midnight.
_Avoid_: score, streak, stats, summary
