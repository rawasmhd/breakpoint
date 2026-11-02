# Toast capabilities for unpackaged Rust apps

Resolves [#27](https://github.com/rawasmhd/breakpoint/issues/27). Feeds the "Notification channel: toast vs popup" decision and backlog ticket 6 (Notifications phase 1).

**Question.** What can an unpackaged (no MSIX, no package identity) Rust `.exe` do with Windows toast notifications on Windows 10 1809+ and Windows 11, and what registration does each capability need?

**Method.** Primary sources only: Microsoft Learn (current pages plus the Win32 toast pages Microsoft archived on 2025-07-28, read from the `MicrosoftDocs/windows-dev-docs` history at commit `dfc8a9c1`), the WinRT API reference, Microsoft Support, the Windows Community Toolkit source (Microsoft's own compat library), and the crates' docs.rs pages, repositories and issue trackers. Every claim carries a link. Where a limitation is Microsoft's, it is quoted.

---

## TL;DR

| You want | Zero registration (PowerShell AUMID) | Own AUMID in registry | Own AUMID + COM activator |
| --- | --- | --- | --- |
| Toast with title, body, up to 5 buttons | Yes | Yes | Yes |
| Correct app name and icon in the toast header | No (shows Windows PowerShell) | Yes | Yes |
| Button click while the app is running (in-process event) | Yes | Yes | Yes |
| Button click from Action Center after the popup is gone, or after the exe exited | No | No | Yes |
| Text input / selection box values | No | No | Yes |
| `scenario="reminder"` (stays on screen until dismissed), `duration="long"` | Yes | Yes | Yes |
| Progress bar / countdown updated in place | Yes | Yes | Yes |
| Persist in Action Center | Not documented for this path | With a stub CLSID | Yes |
| Break through Do Not Disturb | Only if the user allows the app or `scenario="urgent"` on Win 11 | Same | Same |

Recommended crate: **`tauri-winrt-notification` 0.8** for the MVP, driven with our own AUMID registered under `HKCU`, and the underlying **`windows`** crate (already a dependency of it) for anything it lacks, in particular a COM activator in a later milestone. Details in section 7.

---

## 1. Identity: the AppUserModelID (AUMID)

Every toast is sent through `ToastNotificationManager.CreateToastNotifier(applicationId)`. Microsoft's API remarks for desktop apps:

> "Do not use this overload when creating a toast notifier for a desktop app. Use CreateToastNotifier(appID) to supply the required AppUserModelID." ([CreateToastNotifier](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotificationmanager.createtoastnotifier))

and under "Sending toast notifications from desktop apps" on the same page:

> "For a desktop app to display a toast, the app must have a shortcut on the Start screen. The shortcut must have an AppUserModelID. Desktop apps cannot schedule a toast."

The last sentence is stale: Microsoft's current unpackaged guidance documents scheduled toasts and only limits the AUMID length for them (section 1.3). The identity requirement is real, though, and there are three ways to satisfy it.

### 1.1 Option A: borrow the PowerShell AUMID (zero registration)

`{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe` is the AUMID of the Windows PowerShell Start Menu shortcut, which exists on every Windows install. All four crates in section 6 default to it. The crate authors document the catch:

> "This can be used if you do not have a AppUserModelID." ... the toast "will erroneously report its origin as powershell." ([tauri-winrt-notification `src/lib.rs`](https://github.com/tauri-apps/winrt-notification/blob/dev/src/lib.rs); same wording in [allenbenz/winrt-notification](https://github.com/allenbenz/winrt-notification/blob/main/src/lib.rs))

On Windows 11 the header of every toast (the "attribution area") shows the app name and icon and cannot be overridden from XML:

> "Starting with Windows 11, your app's name and icon are displayed in this area. ... The attribution area is configured by the shell and can't be overridden in the toast XML payload" ([App notification content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content))

So with Option A every reminder is titled "Windows PowerShell" with the PowerShell icon, and the per-app notification toggle in Settings is PowerShell's. Nothing else is lost for showing a toast; button clicks still reach the running process (section 3.1). Activation after the process exits has nowhere to go: the registered target for that AUMID is PowerShell's shortcut, not our exe.

### 1.2 Option B: register our own AUMID (registry and/or Start Menu shortcut)

Two documented mechanisms, which can be combined.

**Registry only.** Microsoft's guide for unpackaged apps that are not C#/C++ (exactly our case) registers nothing but registry values:

> "You first need to register your app's information in the registry, including a unique AUMID that identifies your app, your app's display name, your icon, and a COM activator's GUID."
>
> ```
> HKEY_LOCAL_MACHINE\Software\Classes\AppUserModelId\<YOUR_AUMID>
>     DisplayName          REG_EXPAND_SZ  "My App"
>     IconUri              REG_EXPAND_SZ  "C:\icon.png"
>     IconBackgroundColor  REG_SZ         "AARRGGBB"
>     CustomActivator      REG_SZ         "{YOUR COM ACTIVATOR GUID HERE}"
> ```
>
> ([Send a local toast notification from other types of unpackaged apps](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps.md), archived 2025-07-28)

Microsoft's own compat library writes the same key per user instead of per machine, which needs no elevation: `HKCU\Software\Classes\AppUserModelId\<aumid>` with `DisplayName`, `IconUri`, `IconBackgroundColor` (hard-coded `FFDDDDDD`) and `CustomActivator`, plus `HKCU\SOFTWARE\Classes\CLSID\{guid}\LocalServer32` = `"<exe path>" -ToastActivated` ([ToastNotificationManagerCompat.cs](https://github.com/CommunityToolkit/WindowsCommunityToolkit/blob/main/Microsoft.Toolkit.Uwp.Notifications/Toasts/Compat/ToastNotificationManagerCompat.cs)). The toolkit's `[Obsolete]` note on the older class says the new one needs "no Start menu shortcut and no COM GUIDs" from the developer ([DesktopNotificationManagerCompat.cs](https://github.com/CommunityToolkit/WindowsCommunityToolkit/blob/main/Microsoft.Toolkit.Uwp.Notifications/Toasts/Compat/Desktop/DesktopNotificationManagerCompat.cs)). For the icon it extracts the exe icon and saves it as `%LOCALAPPDATA%\ToastNotificationManagerCompat\Apps\<aumid>\Icon.png` ([Win32AppInfo.cs](https://github.com/CommunityToolkit/WindowsCommunityToolkit/blob/main/Microsoft.Toolkit.Uwp.Notifications/Toasts/Compat/Desktop/Win32AppInfo.cs)). The Windows App SDK does the equivalent: "For unpackaged apps, the calling process will be registered as the COM server and assets like the app display name and icon will be retrieved from the shell and registered as well." ([AppNotificationManager.Register](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.appnotifications.appnotificationmanager.register))

**Start Menu shortcut.** The classic route: a `.lnk` under `%APPDATA%\Microsoft\Windows\Start Menu\Programs` whose property store carries `System.AppUserModel.ID` (and, for activation, `System.AppUserModel.ToastActivatorCLSID`):

> "Without a valid shortcut installed in the Start screen or in All Programs, you cannot raise a toast notification from a desktop app." ([How to enable desktop toast notifications through an AppUserModelID](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/legacy/hh802762(v=vs.85)), archived)
>
> ```xml
> <ShortcutProperty Key="System.AppUserModel.ID" Value="YourCompany.YourApp"/>
> <ShortcutProperty Key="System.AppUserModel.ToastActivatorCLSID" Value="{...guid...}"/>
> ```
>
> ([Send a local toast notification from a WRL C++ desktop app](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md), archived)

The shortcut doubles as the "Start with Windows" / Start Menu entry breakpoint wants anyway (ticket 3), and the process should also call `SetCurrentProcessExplicitAppUserModelID` at startup so taskbar grouping matches: "This method must be called during an application's initial startup routine before the application presents any UI" ([SetCurrentProcessExplicitAppUserModelID](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-setcurrentprocessexplicitappusermodelid)).

Caveat for breakpoint: the registry-only path is documented with `CustomActivator` included; Microsoft does not document the combination "registry `DisplayName`/`IconUri` and no `CustomActivator`". Toasts should show (the AUMID is what the notifier needs), but Action Center persistence and the exact header rendering must be verified on a clean VM (ticket 6 AC already requires a VM test).

### 1.3 AUMID rules

> "It can have no more than 128 characters and cannot contain spaces. Each section should be pascal-cased. `CompanyName.ProductName.SubProduct.VersionInformation`" ([Application User Model IDs](https://learn.microsoft.com/en-us/windows/win32/shell/appids))
>
> "The AUMID should be at most 129 characters long. If the AUMID is more than 129 characters long, scheduled toast notifications won't work - you'll get the following exception when adding a scheduled notification: The data area passed to a system call is too small. (0x8007007A)." ([other unpackaged apps](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps.md))

Something like `Rawasmhd.Breakpoint` fits. If a crate lets the AUMID go unregistered, Windows fails silently (tauri-apps/winrt-notification [#17 "Fails silently if app ID is invalid"](https://github.com/tauri-apps/winrt-notification/issues/17)); Microsoft describes the same symptom: "If your notifications simply fail to appear in your desktop app (and no exceptions are thrown), that likely means the Start shortcut isn't present ..., or the AUMID you used in code doesn't match" ([WRL guide](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md)).

---

## 2. Content: title, body, two buttons, images

All of this is plain XML in the `ToastGeneric` binding and needs no registration beyond an AUMID.

- Text: "Each app notification must have at least one text element, and can contain two additional text elements". Title up to 2 lines, the other two up to 4 lines combined ([content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content)).
- Buttons: `<actions><action content='Done' arguments='action=done'/>...</actions>`. "You can only have up to 5 buttons (including context menu items" ([content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content)); the schema allows "up to five inputs and up to five button actions" ([toast element](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-toast)).
- `activationType` on a button is `foreground` (default), `background` or `protocol` ([action element](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-action)). For desktop apps background is meaningless: "For desktop apps, foreground and background activation is handled identically — your COM activator is called. ... specifying an activationType of background in your toast content doesn't change the behavior." ([WRL guide](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md)); the Windows App SDK repeats "Setting activationType="background" in the notification XML payload is ignored for desktop apps." ([quickstart](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-quickstart)).
- Button icons (16x16 white PNG) and, "New in Windows 11 Update", `hint-toolTip` and green/red `hint-buttonStyle` with `useButtonStyle='true'` ([content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content)).
- Images: `file:///` paths are "Supported only for desktop apps" ([image element](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-image)); "Http images are supported only in packaged apps ... Unpackaged apps don't support http images" ([WRL guide](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md)). `placement='appLogoOverride'` puts a 48x48 image at the left, optionally `hint-crop='circle'`, but "In Windows 11, the app logo is displayed in the attribution area, so it is not overridden by the appLogoOverride image placement." ([content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content)). Hero image 364x180 is also available.
- Legacy Windows 8 templates (`ToastText02` etc.) must not be used: "Desktop apps can't use legacy toast templates ... You must use the Windows ToastGeneric templates" ([WRL guide](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md)).
- Payload cap: "the entire XML notification content cannot exceed 5 KB", and an app can hold at most 20 notifications in Notification Center ([headers](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-headers)).

---

## 3. Receiving clicks

### 3.1 In-process, while the app is running: `ToastNotification.Activated`

> "Occurs when user activates a toast notification through a click or touch. Apps that are running subscribe to this event." Remarks: "In the case of toast raised by a desktop app, that app must subscribe to at least the Activated event so that it can handle the expected activation of the app from the toast when the user selects it." ([ToastNotification.Activated](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification.activated))

The handler receives `ToastActivatedEventArgs.Arguments`, the `arguments` string of the clicked button or the toast's `launch` string, "This lets the app tell which action was taken when multiple actions were exposed." `UserInput` (text boxes, selection boxes) was added in Windows 10 1903 / build 18362 ([ToastActivatedEventArgs](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastactivatedeventargs)). Siblings: `Dismissed` ("Occurs when a toast notification leaves the screen, either by expiring or being explicitly dismissed by the user", with `UserCanceled`, `ApplicationHidden`, `TimedOut`) and `Failed` ([Dismissed](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification.dismissed), [ToastNotification](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification)). `Dismissed` with `TimedOut` is how a reminder toast can tell us the user ignored it.

This works with any AUMID, including PowerShell's, and is exactly what `tauri-winrt-notification::Toast::on_activated` wires up (section 6). Two practical constraints follow from the API shape: the handler fires on a WinRT thread pool thread, not the tray message loop, so the app needs a channel back to the main thread; and the event is tied to the live `ToastNotification` object, so the app must keep that object alive until the toast is gone.

### 3.2 After dismissal, from Action Center, or after the exe has exited: COM activator

> "Notifications can be clicked at any point in time, even when your app isn't running. Thus, notification activation is handled through a COM activator. Your COM class must implement the INotificationActivationCallback interface. The GUID for your COM class must match the GUID you specified in the registry CustomActivator value." ([other unpackaged apps](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps.md))
>
> "This is required for your toast to persist in Action Center (since the toast could be clicked days later when your app is closed)." ... "If your notifications appear but aren't persisted in Action Center (disappearing after the popup is dismissed), that means you haven't implemented the COM activator correctly." ([WRL guide](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md))

What a COM activator consists of, all documented in the WRL guide and the toolkit source:

1. A GUID (CLSID) for the activator class.
2. `HKCU\Software\Classes\CLSID\{CLSID}\LocalServer32` default value `"C:\path\breakpoint.exe" -ToastActivated`.
3. The CLSID published as `CustomActivator` in the AUMID registry key and/or `System.AppUserModel.ToastActivatorCLSID` on the shortcut.
4. At startup, `CoRegisterClassObject(CLSID, factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)` for a class implementing `INotificationActivationCallback::Activate(appUserModelId, invokedArgs, NOTIFICATION_USER_INPUT_DATA* data, ULONG count)`. In Rust this is the `windows` crate's `#[implement]` macro over the `Win32::UI::Shell::INotificationActivationCallback` interface.
5. Handling the `-ToastActivated` launch: "If your app is not running: 1. Your app is EXE launched, you get a command line args of "-ToastActivated" 2. Activate in your NotificationActivator is called" ([WRL guide](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md)). For a single-instance tray app this means the mutex guard must let a `-ToastActivated` launch hand the click to the running instance (or the running instance, already registered as the COM server, simply receives the call and no second process is started).

Known wrinkle Microsoft lists: "The Activate method is called on a separate thread from your main thread." Fixed since build 16299: foreground rights now transfer to the activated app ([WRL guide](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md)).

### 3.3 What is lost without a COM activator

Microsoft's own comparison for desktop apps ([Activating toast notifications from desktop apps](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/toast-desktop-apps.md), archived 2025-07-28):

| Option | Visuals | Actions | Inputs | Activates in-process |
| --- | --- | --- | --- | --- |
| COM activator | Supported | Supported | Supported | Supported |
| No COM / Stub CLSID | Supported | Supported | Not supported | Not supported |

and for the no-COM route, per template and activation type, unpackaged: ToastGeneric Foreground "Not supported", Background "Not supported", Protocol "Supported". The page explains the stub trick:

> "set up the AUMID so that you can send toasts, and then also specify a CLSID on your shortcut. That can be any random GUID. Don't add the COM server/activator. You're adding a "stub" COM CLSID, which will cause Action Center to persist the notification. Note that you can use only protocol activation toasts, because the stub CLSID will break activation of any other toast activations."

"Activates in-process: Not supported" in that table is about the shell routing a click into the running process through COM. The `ToastNotification.Activated` event of section 3.1 is a different mechanism: it is delivered to the live WinRT object and keeps working for the popup phase regardless of COM, which is why the Rust crates can offer `on_activated` with the PowerShell AUMID. Taken together, without a COM activator breakpoint:

- gets clicks on "Done" / "Snooze" only while the toast banner is on screen and the process is alive;
- cannot read text or selection inputs (not needed: breakpoint has none);
- cannot receive a click made later from Action Center, and the toast may not even be kept there (section 5.1 for why that matters little for timed reminders);
- can still make a button open a URL or a registered protocol (`activationType='protocol'`), which is a documented workaround for a "Settings" button.

A `-ToastActivated` COM activator is therefore a Phase 2 feature; the MVP in ticket 6 does not need it.

---

## 4. Focus Assist / Do Not Disturb

**Suppression.** Windows 11 Do not disturb (Focus Assist on Windows 10): "you will only receive banners for alarms, reminders, and apps of your choice. Other notifications are sent directly to the notification center until you turn it off." Users can allow specific apps ("Under Apps, select which apps are allowed to send notifications while do not disturb is on by selecting the button Add apps") and tick "allow incoming calls or reminders while do not disturb is on". It turns on automatically "During certain time of the day", "When duplicating the display", "When playing a game", "When using an app in full-screen mode", "For the first hour after a Windows feature update" ([Notifications and Do Not Disturb in Windows](https://support.microsoft.com/en-us/windows/notifications-and-do-not-disturb-in-windows-8942c744-6198-fe56-4639-34320cf9444e); Windows 10 modes Off / Priority only / Alarms only: [Focus assist](https://support.microsoft.com/en-us/windows/turn-focus-assist-on-or-off-in-windows-5492a638-b5a3-1ee0-0c4f-5ae044450e09)). So yes, our toasts are suppressed by default; they land silently in Notification Center. Whether Windows' "reminders" exception means toasts with `scenario="reminder"` is not spelled out in the developer docs; it is plausible but should be tested on a VM rather than assumed.

**Breaking through.** The `urgent` scenario: "This allows users to have more control over what apps can send them high-priority toast notifications that can break through Focus Assist (Do not Disturb)" ([toast element](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-toast)); "Requires: You must be running Windows Insider Preview Build 22546 or later to use important notifications" and the first urgent toast makes Windows ask the user to allow or block them for the app ([content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content)). That is Windows 11 22H2+ only, opt-in by the user, and heavy-handed for a stand-up nudge; a possible option for the end-of-day "stop working" reminder only.

**Reading the state.** There is no documented API that tells an app "Do not disturb is on":

- `ToastNotifier.Setting` returns `NotificationSetting`, whose values are only `Enabled`, `DisabledForApplication`, `DisabledForUser`, `DisabledByGroupPolicy`, `DisabledByManifest` ([NotificationSetting](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.notificationsetting), [ToastNotifier.Setting](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotifier.setting)). Useful to detect "user switched our notifications off", not Focus Assist.
- `Windows.UI.Shell.FocusSessionManager.IsFocusActive` "Gets a value indicating if a focus session is active on the device", requires UniversalApiContract v15 (SDK 22621+, device family "Windows 11 Insider Preview (introduced in 10.0.23504.0)"), and covers the Focus-sessions feature of the Clock app, not a manual DND toggle or automatic rules; starting or stopping sessions is a Limited Access Feature ([FocusSessionManager](https://learn.microsoft.com/en-us/uwp/api/windows.ui.shell.focussessionmanager), [IsFocusActive](https://learn.microsoft.com/en-us/uwp/api/windows.ui.shell.focussessionmanager.isfocusactive)).
- The toast `Dismissed` event does not report suppression; a suppressed toast simply never pops.

Consequence for breakpoint: keep our own "quiet" detection (full-screen foreground window, presentation mode, lock) in the scheduler as SPEC already plans, and treat DND as something Windows handles for us. Document in the README that users who want nudges during DND add breakpoint under Settings > Notifications > Set priority notifications.

---

## 5. Persistence, duration, countdowns, icon and name

### 5.1 How long a toast stays

- `duration="short"` (default) or `"long"` ([toast element](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-toast)). Microsoft's current pages do not state the seconds; the Rust crates document 7 s and 25 s ([tauri-winrt-notification `Duration`](https://github.com/tauri-apps/winrt-notification/blob/dev/src/lib.rs)).
- `scenario="reminder"`: "This will be displayed pre-expanded and stay on the user's screen till dismissed. Note that this will be silently ignored unless there's a toast button action that activates in background." ([toast element](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-toast)); current wording: "You must provide at least one button on your app notification. Otherwise, the notification will be treated as a normal notification." ([content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content)). This is the right scenario for stand/eye breaks: it waits for "Done" or "Snooze".
- `scenario="alarm"` adds looping alarm audio; `incomingCall` is a call layout; neither fits.
- After the popup, toasts live in Notification Center: "The default and maximum expiration time for local toast notifications is 3 days." Set `ExpirationTime` shorter so a stale "stand up" does not linger ([Send a local toast from a C# app](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast.md), [ExpirationTime](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification.expirationtime)). Apps must clear their own history: "When your app is launched, we do NOT automatically clear your notifications" (same page), via `ToastNotificationManager.History.Remove/Clear` ([History](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotificationmanager.history)). For a timer app, removing the previous reminder when the next fires is the sane default, which also makes Action Center persistence (section 3.2) mostly irrelevant.
- `SuppressPopup = true` sends a toast straight to Notification Center without a banner, handy for the "merged" or low-priority water reminder ([ToastNotification](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification)).

### 5.2 Progress bars and countdowns

`<progress title status value valueStringOverride/>` is "Only supported on toasts on Desktop, build 15063 or later" (Windows 10 1703, so fine for 1809+) ([progress element](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-progress)). Values can be bound (`value='{progressValue}'`) and updated in place with `ToastNotifier.Update(NotificationData, tag[, group])`, also 15063+, returning `NotificationUpdateResult` ([Update](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotifier.update)). Updating "Leaves the notification in place", "Won't reappear as a popup", and "If the user dismissed the notification, the update will fail"; replacing (re-`Show` with the same Tag/Group) re-pops and moves it to the top ([progress bar and data binding](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-progress-bar)). Only progress-bar properties and top-level `<text>` elements are bindable, so a 20-second eye-break countdown is: `scenario='reminder'`, a bound progress bar plus a bound text line ("12 s left"), and one `Update` per second with an increasing sequence number. `tauri-winrt-notification` exposes this as `Toast::progress` and `set_progress` (section 6). One crate author notes progress toasts can vanish before completion unless the scenario keeps them on screen ([win-toast-notify `src/lib.rs`](https://github.com/iKineticate/win-toast-notify/blob/main/src/lib.rs)), consistent with the reminder requirement above.

### 5.3 Icon and app name

On Windows 11 both come from the AUMID registration (`DisplayName`, `IconUri`, or the shortcut's name and icon) and cannot be set per toast (section 2). On Windows 10 the `appLogoOverride` image is shown in place of the app logo. The toolkit's choice of a 48x48 PNG extracted from the exe is a good template for breakpoint: ship `breakpoint.png` next to the exe or write it under `%LOCALAPPDATA%\breakpoint\` and point `IconUri` at it.

---

## 6. Crate options

All four wrap the same `Windows.UI.Notifications` WinRT API; none implements a COM activator. Maintenance data is from crates.io and the GitHub API on 2026-11-02.

| Crate | Latest | Released | Last commit | Backend | Buttons | In-process click callback | Progress / update | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| [`windows`](https://docs.rs/crate/windows/latest/features) | 0.62.2 | active (Microsoft) | active | Direct WinRT bindings; features `UI_Notifications`, `Data_Xml_Dom`, `Foundation_Collections` out of 673 | Yes (raw XML) | Yes (`Activated` event) | Yes | Everything the API has, including `INotificationActivationCallback` for a COM activator and `Win32_UI_Shell` for AUMID/shortcut work; most code to write |
| [`tauri-winrt-notification`](https://crates.io/crates/tauri-winrt-notification) | 0.8.1 | 2026-07-17 | 2026-07-17 | `windows` 0.62 (`Win32_Foundation`, `Foundation_Collections`, `Win32_System_SystemInformation`, `Data_Xml_Dom`, `UI_Notifications`), `thiserror`, `windows-version` | `add_button(content, action)` | `on_activated(Fn(Option<String>))` returns the button's `arguments`; `on_dismissed(reason)` | `progress(&Progress)`, `set_progress` | Title, two text lines, icon/hero/image, sound, `Duration`, `Scenario` (Default/Alarm/Reminder/IncomingCall; no Urgent), `POWERSHELL_APP_ID`. No Tag/Group/ExpirationTime, no `UserInput`, no history API. 11.7 M downloads, maintained by the Tauri org; [#59 "buttons and sounds are broken"](https://github.com/tauri-apps/winrt-notification/issues/59) was closed with 0.8.1 |
| [`winrt-notification`](https://crates.io/crates/winrt-notification) | 0.5.1 | 2022-01-11 | 2022-01-11 | `windows` 0.24, `xml-rs`, `strum` | No ([#5 "Actions support?"](https://github.com/allenbenz/winrt-notification/issues/5) open since 2021) | No | No ([#17](https://github.com/allenbenz/winrt-notification/issues/17) open) | The original crate that the Tauri org forked and continued as `tauri-winrt-notification`. Effectively unmaintained |
| [`win-toast-notify`](https://crates.io/crates/win-toast-notify) | 0.1.6 | 2024-08-01 | 2025-03-21 | Spawns `powershell.exe -Command <script>` per toast (`std::process::Command`, `CREATE_NO_WINDOW`), `xml` crate | Yes, protocol activation only | None ("The library cannot report clicks or button presses back to the caller") | Yes via PowerShell | README: "currently in an unstable state", tested on Windows 11 only; interpolates values into a PowerShell string; a PowerShell process per reminder is the wrong cost model for a tray timer |

Windows App SDK (`Microsoft.Windows.AppNotifications`) is the path Microsoft now documents first and it registers unpackaged apps automatically, but it requires the Windows App Runtime to be installed or bootstrapped and C++/WinRT-style projections; it is out of scope for a single static Rust exe. Its documented limitations are nonetheless informative: "Apps running with administrator privileges (elevated) cannot send or receive app notifications." ([overview](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/)) and the "background is ignored" note above.

**Exe size.** The `windows` crate compiles only the namespaces you enable (673 feature flags, [docs.rs features](https://docs.rs/crate/windows/latest/features)) and current versions link Win32 imports through `windows-link`'s `raw-dylib`, so no import libraries are bundled ([Rust reference, raw-dylib](https://doc.rust-lang.org/reference/items/external-blocks.html)). A measurement was attempted in this worktree on the dev machine (`x86_64-pc-windows-gnu`, Rust 1.99, `strip = true`): the hello-world baseline is 872 KB; the `tauri-winrt-notification` build failed in `windows-strings`/`windows-result` with "error calling dlltool 'dlltool.exe': program not found" and, after adding rustup's self-contained `dlltool.exe` to `PATH`, "dlltool could not create import library ... CreateProcess". `raw-dylib` on the GNU target needs a working binutils `dlltool`, which this machine does not have. That is directly relevant to backlog ticket 1 (confirm `windows` builds on the GNU toolchain, or install the MSVC linker): the whole `windows` 0.5x+ family, hence every crate above except `win-toast-notify`, hits this. Measure the delta in CI on `windows-latest` (MSVC) once ticket 1 lands; the expected order of magnitude is a few hundred KB for the XML DOM plus notifications bindings, well inside the 1 MB target when combined with `opt-level = "z"` and LTO.

---

## 7. Summary lists and recommendation

### Works with zero registration (PowerShell AUMID)

- Showing a `ToastGeneric` toast with title, up to two body lines, hero/inline/logo image from a `file:///` path, custom or silent sound.
- Up to 5 buttons with `arguments`; protocol-activation buttons that open a URL or app.
- `scenario="reminder"` (stays until dismissed, needs a button), `duration="long"`, `SuppressPopup`, `ExpirationTime`, Tag/Group for replace/remove, `History.Remove/Clear`.
- Progress bar with in-place `Update` (countdowns), Windows 10 1703+.
- In-process `Activated` (which button, while the banner is up and the exe runs), `Dismissed` (user closed vs timed out), `Failed`.
- Reading `ToastNotifier.Setting` to detect that notifications are disabled for the app or user.
- Cost: the toast says "Windows PowerShell" with PowerShell's icon, and the user's per-app notification toggle is PowerShell's.

### Needs AUMID registration (registry `HKCU\Software\Classes\AppUserModelId\<AUMID>` and/or a Start Menu shortcut with `System.AppUserModel.ID`)

- Our own name and icon in the toast header and in Settings > Notifications; our own entry the user can prioritise for Do not disturb.
- Taskbar/Start identity consistent with the toasts (`SetCurrentProcessExplicitAppUserModelID` plus the shortcut).
- Action Center persistence without a real activator, via a stub `CustomActivator`/`ToastActivatorCLSID` GUID (protocol buttons only).
- Scheduled toasts under our identity (AUMID at most 129 chars).

### Needs a COM activator (`INotificationActivationCallback`, `CLSID\{guid}\LocalServer32` = `"exe" -ToastActivated`, `CoRegisterClassObject` at startup)

- Clicks on the toast or its buttons from Notification Center after the banner is gone, including days later or after the exe was closed (Windows launches the exe with `-ToastActivated`).
- Text-box and selection-box input values (`NOTIFICATION_USER_INPUT_DATA`).
- Full Action Center persistence with working foreground/background button activation.

### Recommendation

1. **Crate:** `tauri-winrt-notification` 0.8 for ticket 6. It is the only maintained wrapper with buttons, `on_activated`, `on_dismissed` and progress updates, it is tiny, and it depends on `windows` 0.62, which breakpoint will pull in anyway for the tray and hooks. Where it falls short (Tag/Group, ExpirationTime, `History`, a `Snooze` selection box later) use `windows` directly on the `ToastNotification` it re-exports, or fork the builder into `platform/toast.rs` (it is about 400 lines).
2. **Identity:** do not ship under the PowerShell AUMID. Register `Rawasmhd.Breakpoint` on first run: write `DisplayName` and `IconUri` under `HKCU\Software\Classes\AppUserModelId\Rawasmhd.Breakpoint` (no elevation, `windows-registry` crate), create the Start Menu shortcut with `System.AppUserModel.ID` when "Start with Windows" or first run creates it, and call `SetCurrentProcessExplicitAppUserModelID` at startup. Remove both on an explicit "Uninstall / reset" action, mirroring `ToastNotificationManagerCompat.Uninstall`. Verify the registry-only variant (no `CustomActivator`) on a clean Windows 11 VM; fall back to adding a stub GUID if toasts fail to persist.
3. **Activation:** Phase 1 relies on the in-process `Activated`/`Dismissed` events with `scenario="reminder"` so the banner waits for Done/Snooze; clear the previous reminder from History when the next one fires. Phase 2 (new ticket) adds the COM activator with `windows`'s `#[implement]` so a click from Notification Center reaches the running instance, and routes `-ToastActivated` launches through the single-instance guard.
4. **Do not disturb:** nothing to read, nothing to fight. Keep the scheduler's own quiet detection; mention the priority-notifications setting in the README; consider `urgent` only for the end-of-day reminder on Windows 11 and only as an opt-in.
5. **Toolchain (ticket 1):** the `windows` crate's `raw-dylib` linking does not build on this machine's `x86_64-pc-windows-gnu` toolchain without a working `dlltool`; decide between installing MinGW binutils or the MSVC Build Tools before ticket 6 starts, and measure the exe size delta in CI.

---

## Sources

Microsoft Learn, current:
- [App notification content](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content)
- [App notification progress bar and data binding](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-progress-bar)
- [App notification headers](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-headers)
- [Schedule an app notification](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-scheduled)
- [Quickstart: Send and Handle App Notifications](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-quickstart)
- [Use app notifications with a console app](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-console)
- [App Notifications Overview](https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/)
- [AppNotificationManager.Register](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.appnotifications.appnotificationmanager.register)
- [AppNotificationDuration](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.appnotifications.builder.appnotificationduration)
- Toast schema: [toast](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-toast), [action](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-action), [image](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-image), [progress](https://learn.microsoft.com/en-us/uwp/schemas/tiles/toastschema/element-progress)
- WinRT reference: [ToastNotification](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification), [ToastNotification.Activated](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification.activated), [ToastNotification.Dismissed](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification.dismissed), [ToastNotification.ExpirationTime](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotification.expirationtime), [ToastActivatedEventArgs](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastactivatedeventargs), [ToastNotificationManager.CreateToastNotifier](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotificationmanager.createtoastnotifier), [ToastNotificationManager.History](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotificationmanager.history), [ToastNotifier.Show](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotifier.show), [ToastNotifier.Update](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotifier.update), [ToastNotifier.Setting](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotifier.setting), [NotificationSetting](https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.notificationsetting), [FocusSessionManager](https://learn.microsoft.com/en-us/uwp/api/windows.ui.shell.focussessionmanager), [FocusSessionManager.IsFocusActive](https://learn.microsoft.com/en-us/uwp/api/windows.ui.shell.focussessionmanager.isfocusactive)
- Win32: [Application User Model IDs](https://learn.microsoft.com/en-us/windows/win32/shell/appids), [SetCurrentProcessExplicitAppUserModelID](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-setcurrentprocessexplicitappusermodelid), [How to enable desktop toast notifications through an AppUserModelID](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/legacy/hh802762(v=vs.85))

Microsoft Learn, Win32 toast pages archived 2025-07-28 (MicrosoftDocs/windows-dev-docs at `dfc8a9c1`):
- [Send a local toast notification from other types of unpackaged apps](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-other-apps.md)
- [Send a local toast notification from a WRL C++ desktop app](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast-desktop-cpp-wrl.md)
- [Activating toast notifications from desktop apps](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/toast-desktop-apps.md)
- [Send a local toast notification from a C# app](https://github.com/MicrosoftDocs/windows-dev-docs/blob/dfc8a9c1a08b4d4e8752133c913d1d6c59415871/hub/apps/design/shell/tiles-and-notifications/send-local-toast.md)

Microsoft Support:
- [Notifications and Do Not Disturb in Windows](https://support.microsoft.com/en-us/windows/notifications-and-do-not-disturb-in-windows-8942c744-6198-fe56-4639-34320cf9444e)
- [Turn Focus assist on or off in Windows](https://support.microsoft.com/en-us/windows/turn-focus-assist-on-or-off-in-windows-5492a638-b5a3-1ee0-0c4f-5ae044450e09)

Microsoft source (Windows Community Toolkit, MIT):
- [ToastNotificationManagerCompat.cs](https://github.com/CommunityToolkit/WindowsCommunityToolkit/blob/main/Microsoft.Toolkit.Uwp.Notifications/Toasts/Compat/ToastNotificationManagerCompat.cs), [DesktopNotificationManagerCompat.cs](https://github.com/CommunityToolkit/WindowsCommunityToolkit/blob/main/Microsoft.Toolkit.Uwp.Notifications/Toasts/Compat/Desktop/DesktopNotificationManagerCompat.cs), [Win32AppInfo.cs](https://github.com/CommunityToolkit/WindowsCommunityToolkit/blob/main/Microsoft.Toolkit.Uwp.Notifications/Toasts/Compat/Desktop/Win32AppInfo.cs)

Crates:
- `windows`: [features](https://docs.rs/crate/windows/latest/features), [windows-rs](https://github.com/microsoft/windows-rs), [Windows::UI::Notifications module docs](https://microsoft.github.io/windows-docs-rs/doc/windows/UI/Notifications/index.html)
- `tauri-winrt-notification`: [crates.io](https://crates.io/crates/tauri-winrt-notification), [docs.rs](https://docs.rs/tauri-winrt-notification/latest/tauri_winrt_notification/), [repo](https://github.com/tauri-apps/winrt-notification), [Cargo.toml](https://github.com/tauri-apps/winrt-notification/blob/dev/Cargo.toml), [src/lib.rs](https://github.com/tauri-apps/winrt-notification/blob/dev/src/lib.rs), issues [#17](https://github.com/tauri-apps/winrt-notification/issues/17), [#59](https://github.com/tauri-apps/winrt-notification/issues/59)
- `winrt-notification`: [crates.io](https://crates.io/crates/winrt-notification), [docs.rs](https://docs.rs/winrt-notification/latest/winrt_notification/), [repo](https://github.com/allenbenz/winrt-notification), issues [#5](https://github.com/allenbenz/winrt-notification/issues/5), [#17](https://github.com/allenbenz/winrt-notification/issues/17)
- `win-toast-notify`: [crates.io](https://crates.io/crates/win-toast-notify), [docs.rs](https://docs.rs/win-toast-notify/latest/win_toast_notify/), [repo](https://github.com/iKineticate/win-toast-notify), [src/lib.rs](https://github.com/iKineticate/win-toast-notify/blob/main/src/lib.rs)
- Rust reference: [external blocks, `raw-dylib`](https://doc.rust-lang.org/reference/items/external-blocks.html)
