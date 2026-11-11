# breakpoint: design system

The look and the voice of breakpoint. [SPEC.md](SPEC.md) says what the app does; this file says how it looks and speaks. Where they disagree on a look, this file wins; on behaviour, the spec wins. Decided in [Restyle: the breakpoint design system](https://github.com/rawasmhd/breakpoint/issues/63); the marks are in [`assets/design/`](../assets/design/).

breakpoint is quiet until a nudge falls due, and the best answer to a nudge is to walk away from the desk. The design follows that: neutral surfaces, one red dot, and a popup that never begs for a click.

The red dot comes from the debugger. In a code editor, a breakpoint is a red dot in the gutter: the place where the program stops on purpose. breakpoint puts that dot in your day.

## 1. Principles

1. **Quiet until needed.** Neutral surfaces, no colour fills in the popup, no motion except the popup's fade and button feedback. A nudge must not feel like an error.
2. **Behaviour over buttons.** The popup has no primary button. Snooze is a secondary button and Skip is a ghost button. The `action` fill appears only in dialogs, on Start and Save.
3. **One dot per surface.** `dot` appears once: the tray icon, the popup header, the wizard header. Never use it for text, borders or states.
4. **Native where Windows draws, ours where we paint.** Checkboxes, radios, time pickers, drop-downs and the tray menu stay native controls. breakpoint owns the layout, the type, the popup and the icons.

## 2. Voice

- Write `breakpoint` in lowercase, also at the start of a sentence and in window captions.
- Use sentence case for every title, button and menu item: "Snooze 5 min", "Turn off until next work day".
- Speak to the user as "you". Short, plain, kind. No exclamation marks, no emoji, no guilt ("You missed 3 breaks" is wrong).
- Say what to do, then how to make it count: "Stand up and move", then "Away 1:59 more and it counts".
- Use the [glossary](../GLOSSARY.md) words exactly: nudge (not reminder or notification), Away (not idle), Skip (not dismiss), Pause (not break).
- Numbers: 24-hour time (`08:00`), minutes as "min" in controls and "minutes" in sentences, `m:ss` in the countdown.
- No em dashes. Use a full stop or a comma.
- Do not praise breakpoint in its own copy. Words such as "calm", "honest", "simply", "gentle" or "smart" about the app are wrong; say what it does. The profile name Gentle and the state Quiet are names, not praise.

| Where | Copy |
| --- | --- |
| Stand/Move popup | **Stand up and move** · Stand, roll your shoulders, look at something far away. |
| Pause popup | **Pause** · Step away from the screen for 5 minutes. |
| Water alone | **Drink some water** |
| Countdown | Away 1:59 more and it counts |
| Tray line | Today: 7 taken, 2 skipped |
| Tooltip | breakpoint: quiet until Sunday 08:00 |

## 3. Colour

| Token | Light | Dark | Use |
| --- | --- | --- | --- |
| `surface` | `#fafaf9` | `#1c1b1a` | Ground of the wizard and the settings window. |
| `surface-raised` | `#ffffff` | `#262422` | Ground of the popup. One step lighter than `surface` in both themes, because the popup floats over other apps. |
| `surface-sunken` | `#f1f0ee` | `#141312` | Dialog footer band, countdown bar track, hover fill of secondary and ghost buttons. |
| `ink` | `#1c1b1a` | `#f2f1ef` | Titles, body text and button labels on every surface. 16:1 light, 13:1 dark on `surface-raised`. |
| `ink-muted` | `#57534f` | `#b3aea8` | The nudge line, the countdown, captions and notes. 7.3:1 light, 7.0:1 dark on `surface` and `surface-raised`. |
| `line` | `#e3e1de` | `#3a3835` | Hairlines, the popup outline on Windows 10, press fill of secondary and ghost buttons. Never the only edge of a control. |
| `line-strong` | `#8c8782` | `#77726d` | Border of secondary buttons and other control edges. At least 3.2:1 on `surface` and `surface-raised`. |
| `dot` | `#e5484d` | `#e5484d` | The breakpoint dot: tray icon, popup header, wizard header. Never text. |
| `action` | `#c8333a` | `#c8333a` | Fill of the one primary button in a dialog: Start, Save. Never in the popup. |
| `action-hover` | `#b02a31` | `#b02a31` | Primary button fill on hover and press. |
| `on-action` | `#ffffff` | `#ffffff` | Label on `action` (5.3:1) and `action-hover` (6.5:1). |
| `stand` | `#b5651d` | `#f0a04b` | Stand/Move glyph and its countdown bar fill. Marks only, never text. |
| `water` | `#2b6cb0` | `#6aaef0` | Water glyph. Marks only, never text. |
| `pause` | `#2f7d5b` | `#5fc495` | Pause glyph and its countdown bar fill. Marks only, never text. |
| `quiet` | `#969696` | `#969696` | The hollow tray ring while quiet or Off. Reads on light and dark taskbars. |
| `focus` | `#1c1b1a` | `#f2f1ef` | Keyboard focus ring in dialogs: solid 2px, 2px off the control. |

- Popup ground is `surface-raised`; dialog ground is `surface`; the dialog footer band is `surface-sunken`.
- All text is `ink` or `ink-muted`. There is no coloured text.
- `stand`, `water` and `pause` colour only their glyph and the countdown bar. The nudge's word is always beside its colour, so colour is never the only signal.
- Control edges use `line-strong`. `line` is for hairlines that carry no meaning.
- The popup follows the Windows app theme (`AppsUseLightTheme`). The wizard and the settings window stay light, because native Win32 controls have no supported dark mode.

## 4. Type

One family: **Segoe UI Variable** on Windows 11, **Segoe UI** on Windows 10. Both ship with Windows, so breakpoint ships no font files.

| Style | Face | Size / line | Weight | Use |
| --- | --- | --- | --- | --- |
| `title-lg` | Segoe UI Variable Display | 28 / 36 | 600 | The wizard heading, once per dialog. |
| `title` | Segoe UI Variable Display | 20 / 28 | 600 | The popup title. |
| `body` | Segoe UI Variable Text | 14 / 20 | 400 | The nudge line, dialog labels, control text. |
| `body-strong` | Segoe UI Variable Text | 14 / 20 | 600 | Section headings, profile names, button labels. |
| `caption` | Segoe UI Variable Text | 13 / 18 | 400 | Countdown, notes under a control, the popup header. |
| `caption-strong` | Segoe UI Variable Text | 13 / 18 | 600 | The digits inside the countdown. Segoe UI figures are tabular, so the line does not jitter each second. |

- Nothing is set under 13px. `caption` is the smallest text.
- Text people read is 7:1 or more on its ground: `ink` everywhere, `ink-muted` only on `surface` and `surface-raised`.
- Build every font from one place in code, from these styles, scaled for the monitor DPI. Do not use the 9 pt dialog template font.

## 5. Spacing, shape and size

Sizes are at 96 DPI and scale with the monitor.

| Token | Value | Use |
| --- | --- | --- |
| `space-1` | 4 | Title to line. |
| `space-2` | 8 | Between buttons. Glyph to title. Between rows inside a section. |
| `space-3` | 12 | Popup header to title. Label to its control. |
| `space-4` | 16 | Popup padding. Popup gap to the work-area corner. Line to countdown, countdown to buttons. |
| `space-6` | 24 | Dialog padding. Between dialog sections. |
| `space-8` | 32 | Wizard heading to its first section. |
| `radius-sm` | 4 | Buttons and the profile options. |
| `radius-md` | 8 | The popup window (`DWMWCP_ROUND` on Windows 11; square on Windows 10). |
| `radius-full` | full | The dot, the tray marks, the countdown bar. |
| `size-popup` | 360 | Popup width. Height follows the content. |
| `size-button` | 32 | Button height. |
| `size-glyph` | 20 | Nudge glyph beside the popup title; 16 beside the Water line. |
| `size-mark` | 8 | The dot in the popup header. |
| `size-bar` | 4 | Countdown bar height. |

- Only the popup has depth: the shadow Windows gives a rounded window. Do not paint a shadow. Dialogs are flat.

## 6. Layout

### Popup

- `size-popup` wide, `space-4` padding, `space-4` from the bottom-right corner of the primary monitor's work area.
- No Windows caption bar: no `WS_CAPTION`, no `WS_SYSMENU`. breakpoint paints its own header and close ×.
- Header row, 18 tall: `dot` at `size-mark`, "breakpoint" in `caption` `ink-muted`, close × (24px hit area) at the right.
- `space-3`, then the nudge glyph (`size-glyph`, `space-2` gap) and the title in `title` `ink`.
- `space-1`, then the line in `body` `ink-muted`, aligned with the title text.
- A riding Water line sits `space-2` under the line: the water glyph at 16px and "Drink some water." in `body` `ink`.
- `space-4`, then the countdown bar (`size-bar`, track `surface-sunken`, fill `stand` or `pause`; a combined nudge uses `pause`), then 6px, then the countdown in `caption`, digits in `caption-strong` `ink`. The bar fills as Away time builds up, is full when the nudge counts, and empties when input returns.
- `space-4`, then the buttons, right-aligned, `space-2` apart: Snooze 5 min (secondary), then Skip (ghost). After the snooze is used, Skip only.
- **Water alone:** one row, 12 by 16 padding: the water glyph and "Drink some water" in `body-strong`. No header, no buttons.
- **Bad-config notice:** the header, then the notice in `body`, no buttons.

### Dialogs (wizard and settings)

- `space-6` padding on a `surface` ground. No group boxes: a section is a `body-strong` heading and the controls under it, `space-6` between sections.
- The wizard opens with one `title-lg` heading, "Pick a rhythm". Each profile option is a native radio with the name in `body-strong` and its numbers under it in `caption` `ink-muted`.
- Labels sit left of their controls, `space-3` apart, on one column. Units ("min") follow the edit boxes in `body`. Notes sit under the control they explain in `caption` `ink-muted`.
- Buttons sit in a footer band of `surface-sunken` with a `line` hairline above it, right-aligned, `space-2` apart: the primary first, then Cancel. Wizard: Start. Settings: Save, Cancel.

### Buttons

| Kind | Rest | Hover | Press | Label | Where |
| --- | --- | --- | --- | --- | --- |
| primary | `action` | `action-hover` | `action-hover` | `on-action` | Start, Save. At least 80 wide. |
| secondary | `surface-raised`, 1px `line-strong` | `surface-sunken` | `line` | `ink` | Snooze 5 min, Cancel, Change… |
| ghost | none | `surface-sunken` | `line` | `ink` | Skip, the popup's close ×. |

- `size-button` tall, `space-3` side padding, `radius-sm`, label in `body-strong`.
- One primary button per window at most, and none in the popup. No colour on a label.
- Popup buttons never show a focus ring, because the popup never takes focus.

## 7. Iconography

- Every mark is built from circles, rounded rectangles and at most one simple polygon, so GDI+ can paint it at any DPI from code. The popup needs no image files.
- **Glyphs** (20px grid): `stand.svg` (a standing figure over a floor line), `water.svg` (a drop), `pause.svg` (two bars). The SVGs use the light-theme inks; in dark, paint them in the dark values of the same tokens.
- **Tray icon**, three states told apart by shape, so they read for colour-blind users and on any taskbar:

| State | Mark | File | When |
| --- | --- | --- | --- |
| On | filled `dot` | `tray-on.svg` | Nudges can fire and nothing is pending. |
| Pending | smaller `dot` inside a `dot` ring | `tray-pending.svg` | An occurrence is pending, popup shown or closed with ×. |
| Quiet or Off | hollow ring in `quiet` | `tray-quiet.svg` | Outside work hours, weekend, or Off. |

- Ship each state as an .ico with 16, 20, 24 and 32 px frames. The exe icon (On) adds 48 and 256 px frames.
- **The mark** is `breakpoint-dot.svg`. There is no wordmark file: set "breakpoint" in lowercase in Segoe UI Variable Display at 600 beside or under the dot, with clear space of half the dot's width.
- The tray menu is the native Windows menu. Style it only through its copy: sentence case, "…" on items that open a window. No badge, number or animation on the tray icon.

## 8. Motion

- The popup fades in over 150 ms and out over 100 ms.
- Buttons and the close × answer the pointer. Hover: the fill changes over 150 ms, and back over 120 ms when the pointer leaves. Press: the fill goes one step further within 90 ms and the label moves down 1px. Release: back to hover.
- The focus ring shows at once, with no transition.
- A Firm repeat shows the popup again the same way. It does not shake, flash or grow.
- When Windows animation is off (`SPI_GETCLIENTAREAANIMATION`), everything changes at once.
- Nothing else moves.

## 9. Applying it to Windows

- One theme module in `src/platform/` holds every token as Win32 values (`COLORREF`, DPI-scaled sizes, `HFONT`s per style and DPI). No window procedure holds a colour or size literal.
- The popup is painted in `WM_PAINT` with GDI+, double-buffered. Rounded corners come from `DwmSetWindowAttribute` with `DWMWCP_ROUND`. GDI+ and DWM are `windows` crate features, not new crates.
- Fonts fall back from Segoe UI Variable to Segoe UI when the Variable face is missing.
- The dialogs keep native controls. They change through fonts (`WM_SETFONT`), colours (`WM_CTLCOLORDLG`, `WM_CTLCOLORSTATIC`), layout and an owner-drawn primary button.
