# Desktop design

The desktop's design contract. Guideline references are to Apple's Human Interface Guidelines, as
pinned in the `design-apple` skill of `devsolux/skills` at revision `ffa4af4e`, cited as
`page.md › Heading`. Anything without a citation is a judgment. The rules shape macOS first; the
same layout runs on Windows and Linux without their window treatment.

## Contract

| | |
|---|---|
| **User and job** | One person keeping a private knowledge space: inspecting relations, asking, and deciding what connects to it |
| **First read** | The current area and its object. Relation Space shows relations, not a JSON document |
| **Primary action** | One per area: Initialize Profile until set up (Restore Access when a credential is lost or refused), then Explore Relations, Ask Huda, Run Operation, Connect |
| **Density** | Balanced: macOS 13 pt body and 28 pt controls (`typography.md`, `accessibility.md`) |
| **Interaction model** | Sidebar and detail (`sidebars.md`), grouped rows, sheets for one short task (`sheets.md › Desktop`) |
| **Narrow windows** | Below 760 pt the sidebar keeps only its icons, as `sidebars.md › Desktop` suggests |
| **Signature** | The amber crossbar of the icon joins the parties of a relation, and marks where you are |
| **Forbidden defaults** | Imitation glass, gradients, card grids, emoji icons, an appearance switch |

The sidebar and grouped forms are deliberately familiar: `branding.md` keeps navigation and
controls standard and puts identity in color, content and a few defining moments. The crossbar is
that moment. It carries meaning, so a negated relation has a broken bar and every relation states
its modality, publication status and staleness in words. A planned relation can never pass for a
fact.

## Tokens

`desktop/theme.rs` holds one semantic palette for each appearance, and nothing draws with a literal
color. Blue comes from the icon's field and amber from its crossbar. A test computes every text
pair, so a change that breaks contrast fails the build:

| Pair | Light | Dark | Needs |
|---|---|---|---|
| Body text on window | 15.46 | 15.63 | 4.5 |
| Secondary text on its quietest surface | 6.05 | 6.94 | 4.5 |
| Placeholder text | 4.53 | 5.25 | 4.5 |
| Primary button | 6.00 | 4.63 | 4.5 |
| Destructive button | 5.66 | 5.66 | 4.5 |
| Crossbar on surface | 4.24 | 8.02 | 3.0 |

Type is the system face with the macOS styles: Title 1 22, Title 2 17, Title 3 15, Body 13 and
Subheadline 11.
SF is read at launch from the system with its text optical size, because its default is the
display cut. Windows uses Segoe UI. Linux keeps the bundled faces, because finding the system's
font needs a dependency this app does not otherwise have.

Status is a glyph and a word as well as a color (`accessibility.md`). Only an observed tool call
earns a client the good tone.

## Keyboard

Command with 1 to 8 selects an area in sidebar order, Command-Comma opens Settings, and Escape
closes a sheet. Tab moves through every control in reading order, Space or Return activates the
one in focus, and one ring shows where focus is, because egui draws a focused button like a
hovered one. A sheet keeps Tab inside it and opens with focus on Cancel, so a stray keypress never
makes a change a person has not read.

## Screen readers

eframe's AccessKit bridge is on: NSAccessibility on macOS, UI Automation on Windows and AT-SPI on
Linux. It stays idle until assistive technology asks, and it talks only to the local system.

What a reader receives is designed, not left to defaults:

- **Order.** A row is read as it is seen, text and then controls. Rows are built that way, which
  is why `row_with_trailing` measures its controls instead of building from the trailing edge.
- **Names.** Every control has one, and a list never repeats one: "Connect Cursor", "More for
  Cursor". Inputs are named, not left to their placeholder.
- **Structure.** Each area has one level 1 heading and titled sections below it. The sidebar is a
  navigation landmark that says where you are, and it keeps its names when it shrinks to icons.
- **Sheets** are modal dialogs named after their task, so nothing behind them is reached.
- **Results announce themselves.** A problem interrupts and anything else waits its turn, because
  that line may be the only sign that something happened.
- **Meaning before detail.** A negated relation is read as "Not, Depends on" before its parties,
  because the broken crossbar that shows this at a glance cannot be heard.

`desktop/accessibility_tests.rs` reads the tree of the real app without a window, so losing any of
this fails the build. That proves what a reader is given, not how it sounds.

## Deliberate gaps

Each of these departs from a guideline, and is recorded rather than hidden.

| Gap | Guideline | Why, and what closes it |
|---|---|---|
| **Not yet heard with a real screen reader** | `accessibility.md` | The tree is tested, but VoiceOver, Narrator and Orca have not been run. Each needs a person listening: on macOS, Command-F5 starts VoiceOver |
| Only an application menu | `designing-for-macos.md › Best practices`, every command in the menu bar | The menu bar has Huda's application menu: About Huda with the description, Settings… (Command-Comma), Hide and Quit. Edit, View and Window menus are not built yet: an Edit menu needs testing with real key handling, because its commands could take keys from text fields |
| Settings is a sidebar area, not a Settings window | `settings.md › Desktop` | Moved off every screen first. A window needs the App menu above |
| No vibrancy | `materials.md` | The sidebar is opaque. A real material needs AppKit, and the guidelines prefer none to an imitation |
| The system accent color is not followed | `sidebars.md › Best practices` | eframe does not expose it. Huda's blue is used instead |
| Reduce Motion and Increase Contrast are not read | `accessibility.md` | eframe does not expose them. Motion is limited to 0.1 s, and surfaces are opaque |
| Role order is alphabetical | Judgment | Roles are unordered in the data, so "Dependency, Dependent" can read backwards. Each role is labelled, so it is never wrong. A reading order would belong in the relation type registry |

## Verification

The tab flows, both appearances, the narrow window and keyboard-only use were driven in the
desktop source on macOS arm64, with an injected-input driver that saves the app's own framebuffer.
The packaged binary was not driven, because macOS denied that session Accessibility and screen
capture. Relations were drawn from sample data that round-trips through the real view types, never
from a live Core answer, and no screen reader has been run.
