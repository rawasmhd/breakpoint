# Research notes: what the evidence says about desk-work breaks

Date: 2026-11-02. This is the evidence base behind the default reminder schedule in [SPEC.md](SPEC.md).

Short version: **breaks help, the exact interval is not settled, and the strongest signal is for short, frequent micro-breaks plus not sitting for long unbroken stretches.** Each claim below is graded by how strong its evidence actually is.

## 1. Sitting and standing

- **Guidance (moderate strength):** The 2016 British Journal of Sports Medicine expert statement for desk workers recommends accumulating **2 hours/day of standing and light activity during work hours, progressing toward 4 hours**, and regularly breaking up seated work with standing or short active breaks. It gives **no fixed interval**. The authors note the evidence is mostly observational and one author had commercial ties to sit-stand desks. Source: [BJSM 2015;49:1357](https://bjsm.bmj.com/content/49/21/1357).
- **The "every 30 minutes" rule (weak strength):** This number appears in commercial and lifestyle content, not in formal guidance. It is a practical heuristic, not a validated threshold. No guideline body has adopted a specific number.
- **Design takeaway:** Default a **stand / move reminder every 30 minutes** (1 to 2 minutes standing), and expose the interval. Frame it as "break up sitting" rather than "the science says 30".

## 2. Eye strain: the 20-20-20 rule

- Every 20 minutes, look at something about 20 feet (6 m) away for 20 seconds.
- **Endorsement (moderate):** Recommended by the American Academy of Ophthalmology and the American Optometric Association as a low-cost habit.
- **Evidence (weak and mixed):** A 2013 study of 795 students found fewer computer-vision-syndrome symptoms among those who periodically refocused at distance. A small 2020 clinic study found no symptom change after 20 days. A 2018 Optometry Times review noted no peer-reviewed study had validated the exact rule. Sources: [Healthline summary](https://www.healthline.com/health/eye-health/20-20-20-rule), [Optometry Times](https://www.optometrytimes.com/deconstructing-20-20-20-rule-digital-eye-strain), [Medical News Today](https://www.medicalnewstoday.com/articles/321536).
- **Design takeaway:** Include a **20-minute eye reminder with a 20-second countdown**, on by default but easy to disable because it is the noisiest reminder. Merge it with the stand reminder when they coincide so the user never gets two toasts at once.

## 3. Micro-breaks and productivity

- **Best evidence in this document:** Albulescu et al. 2022, *PLOS ONE* meta-analysis, 22 samples, N=2,335. Micro-breaks (10 minutes or less) had small but significant effects on **vigor (d=0.36)** and **fatigue (d=0.35)**. Effect on performance was **not significant overall** (d=0.16), and longer breaks helped performance more. Sources: [PLOS ONE e0272460](https://doaj.org/article/c6f37b6ba159469296b2cb968b65505f), [EurekAlert release](https://www.eurekalert.org/news-releases/962663).
- **52/17 (weak):** Comes from DeskTime's 2014 analysis of its own most productive users, not a controlled study. Their 2021 rerun found 112/26 instead, which shows how unstable the number is. Source: [Silicon Canals](https://siliconcanals.com/t-desktime-52-17-work-break-productivity-observational-data/).
- **Pomodoro (25/5) and 90-minute ultradian cycles (weak):** Popular, plausible, supported mainly by secondary sources. Vigilance research does show rest breaks during sustained-attention tasks improve performance versus continuous work.
- **Design takeaway:** Do not hard-code one productivity doctrine. Offer **cadence profiles** (Gentle, Balanced, Strict), Balanced by default. Micro-breaks of 1 to 5 minutes are the evidence-backed unit.

## 4. Working hours and stopping

- **WHO / ILO 2021 (strong, population-level):** Working **55+ hours/week** was associated with a 35% higher stroke risk and 17% higher ischaemic heart disease mortality versus 35 to 40 hours. An estimated 745,000 deaths in 2016 were attributed to long hours. Source: [WHO news release](https://www.who.int/news/item/17-05-2021-long-working-hours-increasing-deaths-from-heart-disease-and-stroke-who-ilo).
- **Design takeaway:** A **"stop working" reminder at a configured end time**, with a warning 15 minutes before, and an optional escalating nudge if the user keeps going. Also a **lunch reminder**. These are clock-based, unlike the interval-based micro-breaks.

## 5. Hydration

- Evidence for a specific sipping interval is weak. General guidance is to drink regularly through the day rather than to a fixed timer.
- **Design takeaway:** Include a **hydration reminder every 60 minutes, optional**, folded into the stand reminder so it adds no notification noise.

## 6. Prior art

| App | Stack | Strengths | Why not just use it |
|---|---|---|---|
| [Stretchly](https://github.com/hovancik/stretchly) | Electron, BSD-2 | Very complete: mini/long breaks, strict mode, pause on idle/DND/lock, app-based pausing, themes, CLI | Electron: large install and memory for a timer. Break overlays are its core model, not toasts |
| [Workrave](https://workrave.org) | C++/GTK, GPL | Most configurable, RSI focused, tracks breaks, exercises | Dated UI, heavy configuration |
| EyeLeo | Windows freeware | Eye-focused, can forbid skipping | Closed source, unmaintained |
| Windows Clock "Focus sessions" | Built in | Pomodoro-style focus timer | No health reminders, no recurring stand/eye nudges |

**Gap this project fills:** a tiny, native Windows tray app with evidence-informed defaults that works with zero configuration, uses normal Windows toasts instead of screen takeovers, stays quiet when you are away or presenting, and treats the end-of-day "stop working" nudge as a first-class feature.

## 7. Resulting default schedule (Balanced profile)

| Reminder | Type | Default | Duration shown | Evidence grade |
|---|---|---|---|---|
| Eyes (20-20-20) | interval | every 20 min | 20 s | weak, endorsed |
| Stand / move | interval | every 30 min | 2 min | moderate |
| Hydrate | interval | every 60 min, merged into stand | n/a | weak, low-cost |
| Longer break | interval | every 90 min | 5 to 10 min | moderate (micro-break meta-analysis) |
| Lunch | clock | 12:30 | n/a | common sense |
| Stop working | clock | 18:00, warn at 17:45 | n/a | strong (long hours harm) |

All intervals count **active time**: the clock pauses when the user is idle, locked, or outside work hours.
