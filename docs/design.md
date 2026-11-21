# OverFilm design tokens

One page for the colours, spacing, type and corner rules that the three OverFilm apps share:
the editor (`crates/ui-egui/src/theme.rs`, `Tokens`), OverFilmSubs (`apps/overfilm-subs/src/ui/theme.rs`,
`Palette`) and the OverShorts panel (`overshorts/panel`, CSS variables; it lives in the OverShorts
pull requests, so its values here are from the UI/UX proposal and still to be checked in code).
This is stage 1 of the UI/UX plan: the values are settled here and in code for the editor panel;
Subs and OverShorts are adjusted in their own stages.

The editor stays Premiere-style dark grey (Premiere compatibility is the goal). Subs and
OverShorts move toward the same rules, not the other way round.

## Rules

| Topic | Rule |
|---|---|
| Accent | One flat accent colour, low saturation. No gradients, glow or purple. Status colours (ok, warning, error) only when they mean something. |
| Surfaces | Layers by lightness and a 1px line, never by shadow. No pure black or white panels in new work. |
| Type | One family (Pretendard / Inter). Three sizes: 12 (note), 14 (body and headings), 20 (rare titles). Two weights: regular and bold. No upper-case English kicker labels, no emoji, no numbered circles. |
| Corners | 4px for controls and groups, 8px for dialogs and menus. No pills. |
| Spacing | 4px grid. Related choices close (4 to 6px), groups apart (8 to 12px). |
| Density | A tool, not a landing page: lists and tables, no three-card rows, no stat rows. |
| Hierarchy | One filled primary button per screen. The rest are outlined or plain text buttons. Rarely used options are folded under "Advanced". |
| Motion | State changes only, 0.12 to 0.18 s, fade or move. No bounce or sparkle. |
| Wording | Say what the button does ("Run"), not a slogan. A disabled choice says why, in one line, under it. |
| Icons | Open-licence or original single-colour icons only, each with an `.attribution` sidecar and an `ATTRIBUTION.md` entry (AGENTS.md section 1). |

## Values today

| Token | Editor (dark) | Subs light | Subs dark | OverShorts |
|---|---|---|---|---|
| Window | `#000000` (`app_bg`) | `#f3f3f3` | `#202020` | dark blue-grey |
| Panel / page | `#1d1d1d` (`panel_bg`) | `#f9f9f9` | `#272727` | dark blue-grey |
| Field | `#0e0e0e` (`field_bg`) | `#ffffff` | `#2d2d2d` | |
| Line | `#303030` (`separator`) | `#e5e5e5` | `#1c1c1c` | |
| Text | `#d1d1d1` | `#1a1a1a` | | |
| Dim text | `#b0b0b0` (`text_dim`) | `#5d5d5d` | `#c5c5c5` | |
| Faint text | `#6e6e6e` (`text_faint`) | `#8a8a8a` | `#9a9a9a` | |
| Accent | `#2f6bdf` (`accent`) | `#005fb8` | `#60cdff` | `#4c8dff` |
| Error | `#dc513d` (`danger`) | `#c42b1c` | `#ff99a4` | |
| Control corner | 4 (`radius_sm`) | 4 | 4 | 14 on cards, 99 on badges |
| Gap | 4 (`gap`) | | | |

Differences to close, in the stages that touch each app: OverShorts card corners 14 to 8 (cards) and
4 (controls), pill badges and toggles to 4px rectangles; Subs accent and the editor accent are
different blues. Which single accent blue (or another colour) is a decision for the project owner;
until it is made, each app keeps its own.

## Editor panel building blocks

`crates/ui-egui/src/panels/overfilm_panel.rs` has the shared pieces other editor panels can reuse:

- `section`: a heading with a 1px line round it, corner `radius_sm`, no fill or shadow.
- `advanced`: a closed collapsing header for the options most people never change.
- `review_gate`: a tick that needs a setting; when off it is greyed with the reason under it.
- Run/Stop: a full-width filled button pinned under the scrolling choices.

## Icons (stage 3)

One line-icon family for all three apps, drawn by us, so there is no third-party licence to track
and no asset file (they are code, not images):

- Grid 16 x 16, stroke 1.25 (whole device pixels in the editor), one colour that follows the text
  colour, small filled marks only where a line would be too thin (play triangle, slider knobs, dots).
- Source of truth: `crates/ui-egui/src/icons.rs` (the editor's `Icon` set, original work, same
  licence as the repository). OverFilmSubs draws the same shapes (`apps/overfilm-subs/src/ui/widgets.rs`,
  `Icon`), and the OverShorts panel has them as SVG paths (`overshorts/panel/src/ui/Icon.tsx`).
  A new icon is drawn in the editor set first and copied with the same coordinates.
- No emoji, no text symbols as icons (▶ ● ■ ▾), no Adobe iconography (AGENTS.md section 1), no
  icon fonts or icon packages. Lucide (ISC) and Phosphor (MIT) were looked at in the UI/UX
  proposal and not needed: the editor set already covers what the OverFilm screens use.
- An icon goes beside a word, not instead of it, except in the folded Subs rail (where the word is
  the tooltip) and the Review play button (which has an `aria-label`).

Where they are used:

| App | Where | Icons |
|---|---|---|
| Editor, OverFilm panel | Group headings | Cut `Razor`, Kills `Search`, Captions `Captions`, Render `Export` |
| | Tools groups | Get ready `Monitor`, Cut `Razor`, Read the match `Search`, Captions `Captions`, Export `Export`, Back up `Undo` |
| | Run steps | `StepWaiting`, `StepRunning`, `StepDone`, `StepSkipped`, `StepFailed` (one circle, a different mark) |
| | Run / Stop | `Play` / `Square` |
| OverFilmSubs | Navigation rail | New `Add`, Recent `Clock`, Voices `Mic`, Glossary `Book`, Settings `Gear` |
| | Expander | `ChevronDown`, turned over when open |
| OverShorts panel | Side menu | Home, New, Queue, Review, Voices, Style, Sources, Settings, theme Sun/Moon |
| | Buttons | play, record, stop, chevron (instead of ▶ ● ■ ▾) |

## Motion (stage 3)

Only state changes move, for 0.12 to 0.15 s, by fading or sliding. No bounce, sparkle, scroll
effects or animation libraries (no GSAP, Lenis, Anime.js, Magic UI).

| App | What moves | Time | Turned off by |
|---|---|---|---|
| Editor | Choices that appear under a tick (Auto Cut, Render, caption review wishes) fade in; a running step's progress bar glides to its new value; folding "Advanced" (egui) | 0.12 s (`theme::ANIMATION_TIME`) | Preferences ▸ Appearance ▸ animations off (time 0) |
| OverFilmSubs | The rail's selected marker slides to the chosen page; the expander chevron turns; the switch knob (stage 2) | 0.15 s (`theme::MOTION`) | — |
| OverShorts | A screen's content fades in when the menu changes; menu icon colour; chevron turns; switch and dialog (stage 2) | 0.12 s (`--ease`) | `prefers-reduced-motion: reduce` |
