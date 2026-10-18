# Assistive technology acceptance

Framework AX/UIA inspection, automated audits and actual screen-reader output
are separate evidence. None alone proves every accessibility requirement or
WCAG conformance. Applications must supply semantic, operable frontend content.

## Actual Windows screen-reader run, 2026-10-18

Platform: Windows 11 Pro x64, build 26200, native WebView2. The official
[NVDA 2026.2 portable distribution](https://github.com/nvaccess/nvda/releases/tag/release-2026.2)
was used with eSpeak NG and Speech Viewer. Its downloaded installer SHA-256 was
`f3f8d29974a88d687b3c4809be192219ec579c5bdabcda5aaf53635288bca824`;
Windows reported a valid NV Access Limited Authenticode signature. The reader
was launched locally and exited normally after testing. No account, add-on or
system startup registration was required.

The following were observed in real NVDA output, with application state checked
independently. Keyboard actions used physical Windows input; UIA actions in the
table used the selected native application's actual provider.

| Application and action | Observed result |
| --- | --- |
| Native accessibility fixture: UIA focus required name | Label, edit role, required state and help read |
| UIA focus invalid email | Invalid-entry state and error description read |
| Physical Space on checkbox | Unchecked state read; actual checkbox became false |
| Native Invoke on Save preferences | Polite live region read `Preferences saved locally` |
| Focus password | Protected edit and bullets; fixture plaintext absent |
| Physical Space on dialog opener | Dialog name/role read; initial focus on Close |
| Physical Escape in dialog | Dialog closed; opener focus and label read |
| Production native example: physical Tab | Name edit label read |
| Physical typing and Enter | Actual Rust IPC greeting displayed and read |
| Physical Tab and arrow in theme select | Combo state read; actual native value changed to Dark |
| Corrected generated production app: field focus | Persistent name label, main landmark and IPC region read |
| Physical Tab then Enter on skip link | Skip link read; main content received focus |
| Physical typing and Enter in generated app | Actual Rust greeting displayed and read through the status region |

The baseline runtime was immutable source commit
`a81444f6c162323c82b2e9ae0b6f3bbd5f358b36` (0.1.12). The corrected template was
compiled as a normal release application against that runtime, **without** the
`native-agent` feature. Its production EXE SHA-256 was
`14a444390b9fefa6168bfd2aaed519d2d509eba89f74017abbfcbebb287513d1`.
Debug activation/session/inspector markers were absent and its PE subsystem
was Windows GUI. These results describe the tested binaries; a later release
candidate still needs its own build and isolation checks.

The published 0.1.12 template provided the negative control: its input had only
a placeholder, Enter did not submit, and the button's visible Rust result was
not read because its generic result element was not a live region. The
corrected template supplies a visible label, semantic form, keyboard submit,
main/section landmarks, skip link and atomic polite status. Real host failures
are displayed as errors, including missing IPC bridges.

Only selected fixture utterances belong in release evidence. Full Speech Viewer
logs and native file-picker images can include unrelated local information and
must remain ignored local artifacts.

## Repeatable starter regression

Build the CLI, generate a project, and run it with native dev mode (or use
`examples/hello_world`). Wait for the CLI's session discovery file, then run:

```powershell
node scripts/verify-starter-accessibility.mjs SESSION OUTPUT node_modules/axe-core/axe.min.js target/debug/rdesktop.exe PROJECT/frontend/index.html
```

Repeat with `dev --headless`. Fourteen groups exercise real Rust success/error
responses, Unicode and Enter submission, native UIA label/value, landmarks and
status, keyboard skip navigation, exact frontend hot reload, reduced motion,
forced colors, 360-pixel width and 200% zoom (180 CSS pixels). All axe rules stay
enabled in four audits; violations and incomplete results fail this verifier.
Hidden runs report actual UIA provider availability without showing the window.

Build the application in release mode without development features, then run:

```powershell
node scripts/verify-release-isolation.mjs OUTPUT PATH/TO/APP.exe
```

It checks the GUI subsystem, rejects development transport/inspector markers,
sets development environment switches as negative controls, and verifies that
neither a session file nor debug URL request appears. Close the owned visible
application normally to complete the check.

For manual NVDA repetition, open Speech Viewer, focus the actual app, perform
one action, then inspect both its state and new speech output. Include a plain
production EXE, not only the debug fixture. Quit NVDA normally afterward.

## Remaining acceptance

Speech Viewer proves emitted utterances, not audible intelligibility. Windows
Narrator, braille hardware, voice control, switch access, physical external
file drops, Windows 10, and non-Windows assistive technologies require separate
runtime acceptance. Forced-color and reduced-motion tests use real engine media
emulation; they do not certify every Windows theme or OS setting transition.
OS dialogs beyond the verified message box and WebView2 file picker also need
their own tests. Hidden providers differ from the full engine AX tree.

The native collection fixture retains axe incomplete contrast results and
independent rendered-color measurements; the starter's zero-incomplete result
does not replace those limits. See [the native contract](NATIVE_AGENT.md).
