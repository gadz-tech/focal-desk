# CLAUDE.md — focal-desk

Conventions for anyone (human or Claude) working this repo:

- **Every function gets a `///` doc comment, no matter how simple** — a brief
  note on how it works. No undocumented functions, including test helpers.
- **focal-core stays pure.** No OS calls, no threads, no clocks. The OS talks
  to it only through `engine::Event` in and `engine::Command` out.
- **Features enter as Event/Command variants or config**, never as functions
  bolted onto platform code. See ARCHITECTURE.md ("Where a feature goes").
- **`index.html` is retired (Ryan, 2026-09-03).** Two generations behind the
  engine and nothing real could be tested with it — it only ever showed windows
  moving. Don't touch it, don't port behavior into it. **The focal-core tests
  are the spec:** a behavioral change lands in the engine with the test that
  pins it. (Mock-first was by design in 2026-08; today's word wins.)
- **Run `cargo test --workspace` before pushing.** CI runs Linux and Windows.
- Decided: clicking the desktop (shell window foreground past dwell) maps to
  `Event::ClearStage`. Clicking the focused window is a no-op (test-pinned).
- The Win32 layer (`focal-win`) is gated on `cfg(windows)`, not a Cargo feature:
  Linux CI compiles an empty crate, Windows compiles the real adapter. It has
  been built and smoke-tested on the 8K panel.
- `win::is_manageable` is the one place that decides what focal-desk owns. Keep
  it conservative — a window that cannot be resized cannot be tiled.

<!-- SHARED-RULES:BEGIN — synced from C:\Dev\SHARED-RULES.md by sync-rules.sh; edit THERE, never here -->
# Shared rules — every repo under C:\Dev

**This file is the single source of truth.** `sync-rules.sh` stamps it between
`SHARED-RULES` markers in every repo's `CLAUDE.md`. Rule changes happen HERE,
then run the sync and commit the touched repos — never edit between the markers
in a repo. (Origin: box/CLAUDE.md; centralized 2026-08-21 to kill the
"same text lives in..." copy-paste pattern.)

## The map

`C:\Dev` is a meta-repo (`github.com/ryangadz/dev-meta`, private) holding
`MAP.md` — the census of what every folder on this machine is (repo / lane
checkout / worktree / disposable), with its remote, branch, and purpose.
Confused about a folder? **Read the map, don't guess.** A folder missing from
the map is a defect: add it or rule it disposable. Whoever edits `MAP.md`
regenerates `MAP.html` (Ryan's visual pair) in the same session.

## Session communication rules (added 2026-07-30)

### No loose ends in prose

Statements like "two files still uncommitted at `<path>`" are **banned**. They are
ambiguous (commit what? when? by whom? with what message?) and they are how work gets
lost. Every piece of pending state lands in exactly one of three places:

1. **Resolved** — Claude does it in-session. This is the default. The target state of
   this whole stack is that Ryan never manually commits, pushes, or copy-pastes anything;
   every manual step Claude hands out is a defect to engineer away.
2. **An explicit action block** — when Ryan must act, it is a fenced block at the END of
   the message containing the exact commands or the exact decision needed, and nothing
   else. Never a sentence buried mid-paragraph.
3. **A tracked file** — the handoff's open-threads list, stating what, where, and who
   acts next.

Questions for Ryan are never embedded in a paragraph. Use the question tool, or a
separate line starting `QUESTION:`. One question at a time, impossible to miss.

### Division of labor: Cowork plans, Code executes (added 2026-07-31)

A Cowork session's deliverable is the **written plan**: files on disk, handoff, a
BUILD-QUEUE/runbook entry naming the branch, the paths, and the definition of done.
Execution — git, installs, merges, verification — belongs to **Claude Code sessions**,
which pick the plan up from those files. Proven live 2026-07-31: a Code session
committed and pushed `gate-family-instance` from the written plan at 17:28, minutes
before Ryan pasted a Cowork-issued action block for the SAME work — a double-execution
collision that is exactly the cut-and-paste error class this split exists to remove.
Action blocks to Ryan are reserved for the few things only he can do (GitHub web steps,
physical hardware, secrets) — never for anything a Code session can run.

**Corollary — one session, one lane.** A session that notices work outside its lane
writes it into the plan for the right lane instead of doing it.

**The proven multiplier (reflector studio, days 1-2; Ryan, 2026-07-31): Cowork drafts
prompts for SEVERAL parallel Code workers with disjoint write sets — one checkout per
worker, lanes like features/export/UI — in `box/WORKER-PROMPTS-<date>.md`. Ryan's
paste burden is one launch line per worker, never streams of git. Cut-and-paste is a
scarce resource: every paste is a chance for artifacts to creep into bash/powershell
and for Ryan to lose the bigger picture across windows.**

### Read before you build

Before starting any work: read the connected folder's `CLAUDE.md` and the newest handoff,
then **verify whether the thing already exists** — by observation, not by what a doc
says. Sessions have repeatedly rebuilt finished work or started it in the wrong folder.
If information is missing, ask directly instead of guessing or rebuilding. "Verified"
means observed, never inferred.

### Plans that cross machines (added 2026-08-07)

**A plan that crosses machines is committed and pushed before its worker launches —
uncommitted plans exist on one disk only.** (Proven 2026-08-07: Worker B fetched
everything and the plan wasn't there.) Push it to a branch the worker's machine
actually fetches — the N100's `~/ryan` pulls `lily`, not `main`.

### Current Ryan overrides previous Ryan (added 2026-08-08)

A recorded decision is a memory aid, never a veto. When today's instruction contradicts
the record, say "this was by design (date, reason)" in one line — then follow today's
instruction and update the record to match. Never block current work on past approvals.

### The drop test (added 2026-08-08)

Any machine may die at any moment and it must not matter. Every folder on every machine
is a pushed clone (private GitHub for personal — OneDrive is retired), or explicitly
listed as disposable (in `MAP.md`). Sessions end with work committed and pushed;
overnight uncommitted state is a defect.

## Doc naming: dated = record, undated = living (added 2026-08-21)

A date in a filename means the file is a **record of that date/window** — day plans,
handoffs, worker prompts, single-run runbooks. Records are never updated after their
window closes; new work goes in a new file. **Living documents** (plans of record,
decision records, strategies) carry **no date in the name**; they open with
`**Opened:** <date>` and accrue dated amendment sections inline — chronology lives in
the sections and in git, so there is no last-updated field to rot. When a dated file
starts accruing amendments past its window, it has changed class: rename it undated
(`git mv` + fix references) in that session. **Why (Ryan, 2026-08-21):** a dated title
on a living doc reads as stale, and two similar docs diverging is how future Ryan
loses track of which one is true when something breaks. One living doc per topic;
records frozen at their window.

## Lanes are full clones, not worktrees (added 2026-08-21)

Proven in the XPS→desk machine copy: every full-clone lane survived; six of seven
git worktrees came out orphaned (a worktree's `.git` file embeds an absolute path to
its parent repo, which a copy or a dead cloud mount silently breaks). New parallel
lanes get their **own clone** on their own branch. Harness-managed throwaway
worktrees (`.claude/worktrees/*`) are exempt — they are disposable by construction.

## Privacy boundaries (added 2026-08-21)

- `C:\Dev\olga` is Olga's. Sessions never read or catalog its contents. When her
  private repo exists, seed its `CLAUDE.md` with this shared block.
- **Public repos never carry this block.** Nothing personal, family-internal, or
  machine-specific lands in one — and this block is all three, so a public repo's
  `CLAUDE.md` is deliberately marked `SHARED-RULES:EXEMPT` instead of stamped.
  Currently public: `ryangadz.github.io` and `vector-studio-llm` (the second
  found unmarked 2026-09-01 — it had no markers at all, so the next sync would
  have stamped Olga's folder, Lily's tablet and the N100 into a public repo).
  **Making a repo public means marking its `CLAUDE.md` EXEMPT in the same
  session.**

## Everything lives under `C:\Dev` — never the user profile (Olga, 2026-08-30)

It applies to **paths sessions recommend**, not just files they write. Olga,
2026-08-30, after a runbook step told her to clone to `%USERPROFILE%\Dev\household`:
the users directory on this computer is never used for Claude sessions.

- **No session ever writes to, clones into, or recommends a path under
  `C:\Users\...`** — not `%USERPROFILE%`, not AppData, not Documents or
  Downloads. Every repo, lane, clone and deliverable lives under `C:\Dev`.
- A per-person lane gets its own `C:\Dev` folder (lanes are full clones):
  Olga's household lane is **`C:\Dev\household-olga`**, alongside Ryan's
  `C:\Dev\household`.
- Writing `%USERPROFILE%` into a runbook, worker prompt or paste block is the
  same defect as writing the file there. Check plans for it before handing
  them over.

### File delivery: connected folders, never the session scratchpad (Ryan, repeatedly; finally filed 2026-08-24)

The narrower case of the same rule. Ryan has told sessions repeatedly not to
deliver files into the Cowork outputs/scratchpad under
`C:\Users\ryang\AppData\...` — he can't find them there, and on 2026-08-24 it
cost him his own resume mid-application.

- **Anything Ryan will open, keep, or submit lands in a connected `C:\Dev`
  folder** (the right repo for the content — personal → `ryan/inbox` if
  unsure), THEN gets presented. The scratchpad is for intermediate work
  only.
- Presenting a file straight from the scratchpad is a defect even though
  the tool allows it.

## Ryan creates repos; sessions push into them (added 2026-09-01)

Sessions do not run `gh repo create` — Ryan makes the repo, a session clones and
populates it. The governing rule is about *sensitivity, not complexity*:
**anything whose failure mode is exposure gets initiated by Ryan personally**,
however simple the step looks. Repo creation for family or personal material is
the canonical case — the account/org a repo lands in is the whole security
boundary, and GitHub base permissions are a *floor*, so a repo created in an org
with read-for-all members is exposed the moment it exists. A session that thinks
it needs a new repo asks for one.

## A cross-repo rule lands HERE in the session that writes it (added 2026-09-01)

Three rules accumulated a "this belongs in `SHARED-RULES.md`; a Code session
ports it" note and none got ported — a **loose end in prose**, the exact failure
class the session-communication rules ban. The root cause is narrower than
"sessions forget": a Cowork session usually has one or two repos mounted and
**cannot reach `C:\Dev\SHARED-RULES.md` at all**, so it has nowhere to put the
rule but the local file. Apply the three-places rule instead: a session that
writes a cross-repo rule it cannot port itself **files a `box/WORKER-PROMPTS-<date>.md`
entry for the port in the same session** — a tracked file naming who acts next —
never a prose IOU inside a `CLAUDE.md`.
<!-- SHARED-RULES:END -->
