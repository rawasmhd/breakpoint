//! PROTOTYPE, throwaway. Native Windows toast as the nudge channel.
//!
//! toast stand|pause [--ps] [--soft] [--repeat S] [--snooze S] [--run S] [--keep-registration]
//! toast water [--ps]
//! toast register | unregister
//!
//! Seconds stand in for active minutes: --repeat 20 means "Firm re-pops every
//! 5 active minutes", --snooze 15 means "Snooze 5 active minutes".
//! Default AUMID is our own, registered under HKCU for the run and removed on a
//! normal exit. --ps borrows the Windows PowerShell AUMID (zero registration).

use std::process::Command;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use windows::core::{h, IInspectable, Interface, HSTRING};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::*;

const BP_AUMID: &str = "rawasmhd.breakpoint.prototype";
const PS_AUMID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";
const TAG: &str = "nudge";
const GROUP: &str = "breakpoint";

enum Ev {
    Activated(u32, String),
    Dismissed(u32, ToastDismissalReason),
    Failed(u32),
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("stand").to_string();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let num = |f: &str, d: u64| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };

    match mode.as_str() {
        "register" => return register(),
        "unregister" => return unregister(),
        _ => {}
    }

    let use_ps = flag("--ps");
    let aumid = if use_ps { PS_AUMID } else { BP_AUMID };
    if !use_ps {
        register();
    }
    let t0 = Instant::now();
    let log = |m: &str| println!("[{:>5.1}s] {m}", t0.elapsed().as_secs_f32());
    log(&format!("mode={mode} aumid={aumid}"));

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid))
        .expect("notifier");
    log(&format!("notifier.Setting = {:?}", notifier.Setting().map(|s| s.0)));
    let (tx, rx) = channel::<Ev>();

    if mode == "water" {
        let toast = water_toast().expect("water toast");
        hook(&toast, &tx, 0);
        notifier.Show(&toast).expect("show");
        log("SHOWN water heads-up: silent, no buttons, should time out");
        history(aumid, &log);
        match rx.recv_timeout(Duration::from_secs(40)) {
            Ok(Ev::Dismissed(_, r)) => log(&format!("water dismissed: {}", reason(r))),
            Ok(_) => log("water: unexpected event"),
            Err(_) => log("water: no dismissed event within 40 s (suppressed by Do not disturb?)"),
        }
        history(aumid, &log);
        cleanup(aumid, use_ps, flag("--keep-registration"));
        return;
    }

    let firm = !flag("--soft");
    let repeat = Duration::from_secs(num("--repeat", 20));
    let snooze = Duration::from_secs(num("--snooze", 15));
    let run = Duration::from_secs(num("--run", 300));
    let (title, body) = if mode == "pause" {
        ("Pause", "Focus period over. Step away for 5 minutes; it counts once you have been away that long.")
    } else {
        ("Stand up and move", "Step away for 2 minutes; it counts once you have been away that long. Shoulders down, look at something far away.")
    };

    let mut gen = 0u32;
    let mut snoozed = false;
    let mut seq = 1u32;
    let mut keep_alive = prompt(title, body, snoozed, firm, &tx, gen).expect("toast");
    notifier.Show(&keep_alive).expect("show");
    log(&format!("SHOWN prompt gen {gen} (Snooze + Skip), {}", if firm { "Firm" } else { "Soft" }));
    history(aumid, &log);
    let mut next_repeat = Instant::now() + repeat;
    let mut snooze_until: Option<Instant> = None;
    let mut last_update = String::new();

    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Ev::Activated(g, a)) => match a.as_str() {
                "snooze" => {
                    log(&format!("CLICK reached app: Snooze (gen {g}) -> pending, re-prompt in {}s with Skip only", snooze.as_secs()));
                    snooze_until = Some(Instant::now() + snooze);
                }
                "skip" => {
                    log(&format!("CLICK reached app: Skip (gen {g}) -> occurrence SKIPPED"));
                    break;
                }
                other => log(&format!("body click (gen {g}, args {other:?}): not a button, stays pending")),
            },
            Ok(Ev::Dismissed(g, r)) => log(&format!("dismissed gen {g}: {} -> still pending", reason(r))),
            Ok(Ev::Failed(g)) => log(&format!("FAILED gen {g}")),
            Err(RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
        let now = Instant::now();
        if t0.elapsed() > run {
            log("run time over: occurrence would expire here");
            break;
        }
        if let Some(u) = snooze_until {
            if now >= u {
                snooze_until = None;
                snoozed = true;
                gen += 1;
                keep_alive = prompt(title, body, snoozed, firm, &tx, gen).expect("toast");
                notifier.Show(&keep_alive).expect("show");
                log(&format!("SHOWN re-prompt after snooze gen {gen} (Skip only, default sound)"));
                next_repeat = now + repeat;
            }
            continue;
        }
        if firm && now >= next_repeat {
            gen += 1;
            keep_alive = prompt(title, body, snoozed, firm, &tx, gen).expect("toast");
            notifier.Show(&keep_alive).expect("show");
            log(&format!("FIRM REPEAT gen {gen}: replaced same tag, re-pops with default sound"));
            history(aumid, &log);
            next_repeat = now + repeat;
            continue;
        }
        // In-place countdown update: no re-pop, no sound.
        let left = next_repeat.saturating_duration_since(now).as_secs();
        seq += 1;
        let r = update(&notifier, firm, left, repeat.as_secs(), seq);
        if r != last_update {
            log(&format!("in-place update result: {r}"));
            last_update = r;
        }
    }
    drop(keep_alive);
    cleanup(aumid, use_ps, flag("--keep-registration"));
}

fn prompt(title: &str, body: &str, snoozed: bool, firm: bool, tx: &Sender<Ev>, gen: u32) -> windows::core::Result<ToastNotification> {
    let snooze_btn = if snoozed {
        ""
    } else {
        r#"<action content="Snooze 5 min" arguments="snooze" activationType="foreground"/>"#
    };
    let progress = if firm {
        r#"<progress title="Firm" value="{v}" valueStringOverride="{vs}" status="{s}"/>"#
    } else {
        ""
    };
    // No <audio> element = Windows default notification sound.
    let xml = format!(
        r#"<toast scenario="reminder" launch="body"><visual><binding template="ToastGeneric"><text>{title}</text><text>{body}</text>{progress}</binding></visual><actions>{snooze_btn}<action content="Skip" arguments="skip" activationType="foreground"/></actions></toast>"#
    );
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let t = ToastNotification::CreateToastNotification(&doc)?;
    t.SetTag(&HSTRING::from(TAG))?;
    t.SetGroup(&HSTRING::from(GROUP))?;
    if firm {
        let d = NotificationData::new()?;
        let v = d.Values()?;
        v.Insert(h!("v"), h!("1"))?;
        v.Insert(h!("vs"), h!(""))?;
        v.Insert(h!("s"), h!("re-pops while pending"))?;
        d.SetSequenceNumber(1)?;
        t.SetData(&d)?;
    }
    hook(&t, tx, gen);
    Ok(t)
}

fn water_toast() -> windows::core::Result<ToastNotification> {
    let xml = r#"<toast><visual><binding template="ToastGeneric"><text>Water</text><text>Have a glass of water.</text></binding></visual><audio silent="true"/></toast>"#;
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let t = ToastNotification::CreateToastNotification(&doc)?;
    t.SetTag(h!("water"))?;
    t.SetGroup(&HSTRING::from(GROUP))?;
    Ok(t)
}

fn hook(t: &ToastNotification, tx: &Sender<Ev>, gen: u32) {
    let a = tx.clone();
    t.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
        let s = args
            .as_ref()
            .and_then(|i| i.cast::<ToastActivatedEventArgs>().ok())
            .and_then(|e| e.Arguments().ok())
            .map(|h| h.to_string())
            .unwrap_or_default();
        let _ = a.send(Ev::Activated(gen, s));
        Ok(())
    }))
    .unwrap();
    let d = tx.clone();
    t.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, args| {
        if let Some(r) = args.as_ref().and_then(|e| e.Reason().ok()) {
            let _ = d.send(Ev::Dismissed(gen, r));
        }
        Ok(())
    }))
    .unwrap();
    let f = tx.clone();
    t.Failed(&TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, _| {
        let _ = f.send(Ev::Failed(gen));
        Ok(())
    }))
    .unwrap();
}

fn update(n: &ToastNotifier, firm: bool, left: u64, total: u64, seq: u32) -> String {
    if !firm {
        return "n/a (Soft has no repeat countdown)".into();
    }
    let go = || -> windows::core::Result<NotificationUpdateResult> {
        let d = NotificationData::new()?;
        let v = d.Values()?;
        v.Insert(h!("v"), &HSTRING::from(format!("{:.2}", left as f32 / total.max(1) as f32)))?;
        v.Insert(h!("vs"), &HSTRING::from(format!("0:{left:02}")))?;
        v.Insert(h!("s"), h!("until it pops again"))?;
        d.SetSequenceNumber(seq)?;
        n.UpdateWithTagAndGroup(&d, &HSTRING::from(TAG), &HSTRING::from(GROUP))
    };
    match go() {
        Ok(r) if r == NotificationUpdateResult::Succeeded => "Succeeded".into(),
        Ok(r) if r == NotificationUpdateResult::NotificationNotFound => "NotificationNotFound (toast is gone)".into(),
        Ok(r) => format!("Failed ({})", r.0),
        Err(e) => format!("error {e}"),
    }
}

fn reason(r: ToastDismissalReason) -> &'static str {
    if r == ToastDismissalReason::UserCanceled {
        "UserCanceled (closed without a button)"
    } else if r == ToastDismissalReason::ApplicationHidden {
        "ApplicationHidden (replaced or removed by the app)"
    } else if r == ToastDismissalReason::TimedOut {
        "TimedOut (banner went to Notification Center)"
    } else {
        "unknown"
    }
}

fn history(aumid: &str, log: &dyn Fn(&str)) {
    let r = (|| -> windows::core::Result<String> {
        let h = ToastNotificationManager::History()?.GetHistoryWithId(&HSTRING::from(aumid))?;
        let mut tags = vec![];
        for t in h {
            tags.push(format!("{}/{}", t.Group()?, t.Tag()?));
        }
        Ok(format!("{} in Notification Center {:?}", tags.len(), tags))
    })();
    match r {
        Ok(s) => log(&format!("history: {s}")),
        Err(e) => log(&format!("history: error {e}")),
    }
}

fn cleanup(aumid: &str, use_ps: bool, keep: bool) {
    if let Ok(h) = ToastNotificationManager::History() {
        let _ = h.RemoveGroupWithId(&HSTRING::from(GROUP), &HSTRING::from(aumid));
    }
    if !use_ps && !keep {
        std::thread::sleep(Duration::from_millis(500));
        unregister();
    }
}

fn key() -> String {
    format!(r"HKCU\Software\Classes\AppUserModelId\{BP_AUMID}")
}

fn register() {
    let icon = concat!(env!("CARGO_MANIFEST_DIR"), r"\..\assets\breakpoint.png");
    let icon = std::fs::canonicalize(icon)
        .map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_string())
        .unwrap_or_default();
    let k = key();
    reg(&["add", &k, "/v", "DisplayName", "/t", "REG_SZ", "/d", "breakpoint", "/f"]);
    if !icon.is_empty() {
        reg(&["add", &k, "/v", "IconUri", "/t", "REG_SZ", "/d", &icon, "/f"]);
    }
    println!("registered {k} (icon {icon:?})");
}

fn unregister() {
    reg(&["delete", &key(), "/f"]);
    println!("unregistered {}", key());
}

fn reg(a: &[&str]) {
    let _ = Command::new("reg").args(a).output();
}
