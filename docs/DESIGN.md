# breakpoint: design system

The look and the voice of breakpoint. [SPEC.md](SPEC.md) says what the app does; this file says how it looks and speaks. Where they disagree on a look, this file wins; on behaviour, the spec wins. Decided in [Restyle: the breakpoint design system](https://github.com/rawasmhd/breakpoint/issues/63); the marks are in [`assets/design/`](../assets/design/).

breakpoint is quiet until a nudge falls due, and the best answer to a nudge is to walk away from the desk. The design follows that: neutral surfaces, one red dot, and a popup that never begs for a click.

The red dot comes from the debugger. In a code editor, a breakpoint is a red dot in the gutter: the place where the program stops on purpose. breakpoint puts that dot in your day.

## 1. Principles

1. **Quiet until needed.** Neutral surfaces, no colour fills in the popup, no motion except the popup's fade, control feedback and the switch knob. A nudge must not feel like an error.
2. **Behaviour over buttons.** The popup has no primary button. Snooze is a secondary button and Skip is a ghost button. The `action` fill appears only in dialogs, on Start and Save.
3. **One dot per surface.** `dot` appears once: the tray icon, the popup header, and the app icon in a dialog's caption. Never use it for text, borders or states.
4. **breakpoint paints its controls.** Windows draws the window frame, the tray menu and message boxes. Every control inside the wizard and the settings window is painted by breakpoint.

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
| `surface` | `#fafaf9` | `#1c1b1a` | Ground of the wizard and the settings window. Also `on-ink`: the label or mark on a control filled with `ink`. |
| `surface-raised` | `#ffffff` | `#262422` | Ground of the popup, of the cards in dialogs and of controls at rest. One step lighter than `surface` in both themes. |
| `surface-sunken` | `#f1f0ee` | `#141312` | Dialog footer band, countdown bar track, segmented toggle track, hover fill of buttons, day pills, stepper zones and menu items. |
| `ink` | `#1c1b1a` | `#f2f1ef` | Titles, body text and button labels on every surface. Fill and border of a selected or on control. 16:1 light, 13:1 dark on `surface-raised`. |
| `ink-muted` | `#57534f` | `#b3aea8` | The nudge line, the countdown, captions and notes. 7.3:1 light, 7.0:1 dark on `surface` and `surface-raised`. |
| `line` | `#e3e1de` | `#3a3835` | Hairlines, card outlines, the popup outline on Windows 10, press fill of buttons and stepper zones. Never the only edge of a control. |
| `line-strong` | `#8c8782` | `#77726d` | Border of buttons and every other control at rest, and the knob of an off switch. At least 3.2:1 on `surface` and `surface-raised`. |
| `dot` | `#e5484d` | `#e5484d` | The breakpoint dot: tray icon, popup header, the app icon in dialog captions. Never text. |
| `action` | `#c8333a` | `#c8333a` | Fill of the one primary button in a dialog: Start, Save. Never in the popup. |
| `action-hover` | `#b02a31` | `#b02a31` | Primary button fill on hover and press. |
| `on-action` | `#ffffff` | `#ffffff` | Label on `action` (5.3:1) and `action-hover` (6.5:1). |
| `stand` | `#b5651d` | `#f0a04b` | Stand/Move glyph and its countdown bar fill. Marks only, never text. |
| `water` | `#2b6cb0` | `#6aaef0` | Water glyph. Marks only, never text. |
| `pause` | `#2f7d5b` | `#5fc495` | Pause glyph and its countdown bar fill. Marks only, never text. |
| `quiet` | `#969696` | `#969696` | The hollow tray ring while quiet or Off. Reads on light and dark taskbars. |
| `focus` | `#1c1b1a` | `#f2f1ef` | Keyboard focus ring in dialogs: solid 2px, 2px off the control. It shows only after keyboard use. |

- Popup ground is `surface-raised`; dialog ground is `surface`; a card in a dialog is `surface-raised`; the dialog footer band is `surface-sunken`.
- A selected or on control fills with `ink`, and its label or mark is `surface`. In prose this is `on-ink`. It is the `surface` token, not a new colour. Red stays for the dot and the one primary button.
- All text is `ink` or `ink-muted`. There is no coloured text.
- `stand`, `water` and `pause` colour only their glyph and the countdown bar. The nudge's word is always beside its colour, so colour is never the only signal.
- Control edges use `line-strong`. `line` is for hairlines that carry no meaning.
- The popup, the wizard and the settings window all follow the Windows app theme (`AppsUseLightTheme`). A change of theme repaints an open dialog at once.

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
| `space-2` | 8 | Between buttons. Glyph to title. A dialog section heading to its card. |
| `space-3` | 12 | Popup header to title. Label to its control. Top and bottom padding of a card row. |
| `space-4` | 16 | Popup padding. Popup gap to the work-area corner. Line to countdown, countdown to buttons. Side padding of a card. |
| `space-6` | 24 | Dialog padding. Between dialog sections. |
| `space-8` | 32 | Wizard heading to its first section. |
| `radius-sm` | 4 | Buttons, steppers, the drop-down, checkboxes and menu items. |
| `radius-md` | 8 | The popup window and the drop-down menu (`DWMWCP_ROUND` on Windows 11; square on Windows 10), cards and selectable cards. |
| `radius-full` | full | The dot, the tray marks, the countdown bar, switches and day pills. |
| `size-popup` | 360 | Popup width. Height follows the content. |
| `size-button` | 32 | Button height. |
| `size-glyph` | 20 | Nudge glyph beside the popup title; 16 beside the Water line. |
| `size-mark` | 8 | The dot in the popup header. |
| `size-bar` | 4 | Countdown bar height. |
| `shadow-popup` | system | The shadow Windows gives a rounded floating window. The popup and the drop-down menu have it. |

- Only the popup and the drop-down menu have depth: `shadow-popup`, given by Windows. Do not paint a shadow. Dialogs, cards and controls are flat.

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

- `space-6` padding on a `surface` ground. The system caption and frame stay; breakpoint paints the client area.
- A section is a heading in `body-strong` and a card under it. The card starts `space-2` below the heading. `space-6` separates one section from the next. There are no group boxes.
- **Card:** `surface-raised`, 1px `line` border, `radius-md`, `space-4` padding left and right. Each row has `space-3` above and below its content. A 1px `line` hairline separates two rows.
- Labels sit in one column, 104 wide, left of their controls. Units ("min") follow a stepper in `body`. A note sits under what it explains, in `caption` `ink-muted`.
- The wizard opens with one `title-lg` heading, "Pick a rhythm". Under it, the three profiles are selectable cards: the radio mark, the name in `body-strong`, the numbers in `caption` `ink-muted`, and a small day timeline on the right. Start with Windows is a checkbox row.
- The settings window has three sections: Work week, Nudges (the profile drop-down is its first row) and General (Start with Windows). Its controls are in the Controls table below.
- Buttons sit in a footer band of `surface-sunken` with a `line` hairline above it, right-aligned, `space-2` apart: the primary first, then Cancel. Wizard: Start. Settings: Save, Cancel.

### Controls

Sizes are at 96 DPI. `on-ink` is the `surface` token. The "Press / on" column is the pressed state for a button or a stepper zone, and the on or selected state for the other controls. In high contrast, see §9.

| Control | Size and shape | Rest | Hover | Press / on | Focus |
| --- | --- | --- | --- | --- | --- |
| Button | As Buttons below. | As Buttons below. | As Buttons below. | As Buttons below. | Ring. |
| Switch | 40 × 22, `radius-full`. Knob 14, 3 from the edge. | Off: `surface-raised` track, 1px `line-strong` border, `line-strong` knob. | 1px `ink` border, knob 16. | On: `ink` track and border, `on-ink` knob. The knob slides over 150 ms. | Ring. |
| Checkbox | 18 × 18, `radius-sm`, 1.5px border. | `surface-raised`, `line-strong` border. | `ink` border. | On: `ink` fill, check mark in `on-ink`. | Ring. |
| Day pill | 52 × 32, `radius-full`. Label in `caption-strong`. | `surface-raised`, 1px `line-strong` border, `ink` label. | `surface-sunken` fill. | On: `ink` fill and border, `on-ink` label. | Ring. |
| Stepper | 32 tall, `radius-sm`, 1px `line-strong` border, `surface-raised`. The − and + zones are 28 wide with 12px marks. The value is 56 wide: the number in `body-strong` (tabular), the unit in `body`. | Marks `ink-muted`. | Zone fills `surface-sunken`, mark `ink`. | Zone `line`. | Ring around the field. |
| Time stepper | As Stepper. The value reads `HH:mm`. | As Stepper. | As Stepper. | As Stepper. | As Stepper. |
| Segmented toggle | Track 32 tall, `surface-sunken`, padding 2, gap 2, radius `radius-sm` + 2. Segments 28 tall, at least 50 wide, label in `caption-strong`. | `ink-muted` label. | `ink` label. | Selected: `surface-raised` segment, 1px `line` ring, `ink` label. | Ring around the track. |
| Drop-down | 32 tall, `radius-sm`, 1px `line-strong` border, `surface-raised`. Value in `body`. Chevron 12px in `ink-muted`, 10 from the right. | As the size column. | `surface-sunken` fill. | Open: 1px `ink` border plus a 1px `ink` ring. | Ring. |
| Menu | An owned popup window: `surface-raised`, 1px `line`, `radius-md` (`DWMWCP_ROUND`), `shadow-popup`, padding 4. Items `radius-sm`, padding 8 by 10. Name in `body-strong`, an optional line under it in `caption` `ink-muted`. A 14px check at the left of the current item. | Plain item. | Item `surface-sunken`. | None. | Item `surface-sunken`. |
| Selectable card | `surface-raised`, 1px `line`, `radius-md`, padding 14 by 16. Radio mark 18, 1.5px border, an 8px `ink` dot when selected. The caller paints the content on the right. | `line-strong` radio border. | `ink` radio border. | Selected: 1px `ink` border plus a 1px `ink` ring. | Ring. |
| Card | `surface-raised`, 1px `line`, `radius-md`, `space-4` side padding, hairlines between rows. | As the size column. | None. | None. | None. |
| Text | Any type style, in `ink` or `ink-muted`. | As the size column. | None. | None. | None. |
| Focus ring | 2px `ink` (the `focus` token), 2px gap, following the shape of the control. | Hidden. | None. | None. | Shown after keyboard use, with no transition. |
| Scrollbar | Only on a dialog too tall for its work area. A thumb at the right edge, 2 in from the edge and from the ends of the scrolling part, at least 32 long, `radius-full`, on no track. | 4 wide, `line-strong`. | 8 wide. | Dragged: 8 wide. A click off the thumb pages toward it. | None. |

- The drop-down menu opens below the field, left edges lined up. When there is no room below, it opens above. A menu wider than its field that would hang past the window's right edge lines up its right edge with the field's instead.
- The ring shows only after keyboard use, as Windows does with `UISF_HIDEFOCUS`. A click does not show it.
- A group of selectable cards is one Tab stop, and so is a segmented toggle. Arrow keys move the selection.
- A typed stepper value outside its range stays as typed, so Save can name it ([SPEC.md](SPEC.md) §9.2).
- A number typed in a stepper takes effect as it is typed, so the profile drop-down shows Custom at once. Text that is no number gives back the value from before the typing.
- Enter presses the default button, also on a closed drop-down; Space and Alt+Down open the drop-down. One Esc presses Cancel, also while typing in a stepper.
- A dialog taller than its monitor's work area takes the work area's height. The part between the caption and the footer band scrolls; the band stays in view. The wheel scrolls it by the system's lines per notch, one `body` line each. Keyboard focus scrolls the focused control into view with `space-4` of room. The fit is checked again on a DPI change, a move to another monitor and a work-area change. A dialog that fits does not scroll and shows no scrollbar.

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
- The switch knob slides over 150 ms. Day pills, checkboxes, steppers, the segmented toggle and the drop-down answer the pointer as buttons do: hover over 150 ms in and 120 ms out, press within 90 ms.
- The drop-down menu appears and goes at once, with no fade.
- The focus ring shows at once, with no transition.
- A Firm repeat shows the popup again the same way. It does not shake, flash or grow.
- When Windows animation is off (`SPI_GETCLIENTAREAANIMATION`), nothing moves: everything changes at once, and the switch knob jumps to its place.
- Nothing else moves.

## 9. Applying it to Windows

- One theme module in `src/platform/` holds every token as Win32 values (`COLORREF`, DPI-scaled sizes, `HFONT`s per style and DPI). No window procedure holds a colour or size literal.
- The popup is painted in `WM_PAINT` with GDI+, double-buffered. Rounded corners come from `DwmSetWindowAttribute` with `DWMWCP_ROUND`. GDI+ and DWM are `windows` crate features, not new crates.
- Fonts fall back from Segoe UI Variable to Segoe UI when the Variable face is missing.
- Each dialog is one window, painted with GDI+ in `WM_PAINT` and double-buffered, like the popup. It has no child controls. The layout is a list of items with places at 96 DPI, scaled for the monitor DPI.
- The system caption follows the theme: `DWMWA_USE_IMMERSIVE_DARK_MODE` in dark, and on Windows 11 `DWMWA_CAPTION_COLOR` = `surface`, `DWMWA_TEXT_COLOR` = `ink` and `DWMWA_BORDER_COLOR` = `line`.
- In high contrast, every control paints with system colours instead of tokens: `COLOR_WINDOW`, `COLOR_WINDOWTEXT`, `COLOR_HIGHLIGHT`, `COLOR_HIGHLIGHTTEXT`, `COLOR_BTNFACE` and `COLOR_GRAYTEXT`. Nothing has a shadow.
- Keyboard, focus and screen-reader support are breakpoint's own work. UI Automation exposes every control with a name, a role and its state.
