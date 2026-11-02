# breakpoint

Vocabulary for a Windows tray app that nudges a desk worker to stand, drink water and stop working. One context; this file is the whole glossary.

## Language

**Nudge**:
A recurring prompt to do one healthy thing: stand and move, drink water, stop working. The unit of everything breakpoint does.
_Avoid_: reminder, alert, notification (that is the OS mechanism, not the thing)

**Occurrence**:
One firing of a nudge. A nudge has many occurrences a day; each is satisfied, snoozed or skipped.
_Avoid_: instance, event, alarm

**Cadence**:
How often a nudge fires, measured in active time.
_Avoid_: interval, frequency, schedule

**Active time**:
Time the user is actually at the computer. Idle, locked and asleep periods do not count, so a cadence of 30 minutes means 30 minutes of being at the keyboard.
_Avoid_: uptime, wall-clock time, elapsed time

**Work day**:
A day on which breakpoint is on, together with the hours the user works on it. For Rawas: Sunday to Thursday, 08:00 to 17:00.
_Avoid_: schedule, shift, business hours

**Weekend**:
The days the user does not work and breakpoint stays silent. Not necessarily Saturday and Sunday.
_Avoid_: days off, holidays

**Profile**:
A named preset of cadence numbers: Gentle, Balanced or Strict. Picking one pre-fills the numbers; every number stays individually editable afterwards.
_Avoid_: mode, preset, level

**Enforcement level**:
How insistent a nudge is. **Soft** prompts once and can be ignored. **Firm** keeps prompting until the occurrence is satisfied. **Hard** (not in v1) blocks the screen for the break.
_Avoid_: strictness, severity, priority

**Snooze**:
Pushing an occurrence a few minutes into the future without satisfying it.
_Avoid_: postpone, delay, remind later
