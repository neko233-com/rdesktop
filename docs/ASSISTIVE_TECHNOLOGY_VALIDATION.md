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

The 0.1.13 candidate repeated the last three checks on a fresh CLI-generated
project compiled against the local 0.1.13 crates, without `native-agent`.
NVDA read `Your name`, the main landmark and `Skip to content`; keyboard
activation navigated to `#main`, then Tab reached the input. Native typing of
`Reader final` and Enter produced and announced `Hello, Reader final!` from
the actual Rust handler. This production EXE SHA-256 was
`9453561b7b80f56264a7c6dce48cb96fccb474d814f0a14a5dac71d01a987878`.
Its independent isolation run verified GUI subsystem, zero development
markers, no debug navigation/session despite activation environment variables,
and normal exit. The exact embedded template SHA-256 was
`dd5a4a81e6de1ec4a7ed503a63b0f5a80a28ef7a892b2a05fa548425d3f48748`.

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

## Actual Windows Narrator acceptance, 2026-10-18

Windows Narrator `10.0.26100.8972` was also run on build 26200. Its actual
Speech Recap was inspected visually after individual actions; application
state was checked independently. Narrator was exited through its normal
Home **Exit Narrator** control. No system startup setting was changed.
Microsoft documents Speech Recap in the
[complete Narrator guide](https://support.microsoft.com/en-us/accessibility/windows/narrator/complete-guide-to-narrator).

The same production EXE with SHA-256
`9453561b7b80f56264a7c6dce48cb96fccb474d814f0a14a5dac71d01a987878`
passed four actual reader cases: physical Tab read the skip link, Enter
reached the main landmark, the name field's label and help were read, and
typing `Reader narrator` then Enter produced the actual Rust result
`Hello, Reader narrator!`, which the reader emitted through the live status.
Its separate production-isolation check passed with zero development
markers, no session/debug navigation, and normal exit.

Eight fixture observations covered required label/help, the email negative
control, the corrected email, checkbox Space, protected password, native
Invoke status, modal opening and Escape focus return. Narrator initially
read the email's invalid state without its correction when only
`aria-errormessage` supplied the relationship. Adding `aria-describedby`
made it read `Enter a valid email address.` while retaining invalid state.
NVDA 2026.2 was then repeated on this example. Supplying both associations
made its actual Speech Viewer repeat the correction twice. The final fixture
uses only `aria-describedby` with `aria-invalid`; NVDA's focused-item output
read the correction once. Narrator was restarted and repeated against that
final single-association fixture; it also read the correction once with the
invalid state. This is a compatibility correction in the example,
not a synthesized reader announcement. Native inspection confirms the correction is published as
`full_description`, despite empty `help_text` and `described-by` array.

Native Invoke emitted `Preferences saved locally`. Physical Space opened
the modal and Narrator emitted its name, dialog role, close button and body.
Escape closed it and returned focus to the opener, whose name was read.
Password focus required explicit native ScrollIntoView before SetFocus;
the offscreen provider's failed HRESULT was retained. After scrolling,
the reader emitted protected bullets and UIA value remained redacted.
These selected utterances do not certify audible quality or braille devices.

Run `scripts/verify-native-uia-relations.mjs` for repeatable native description
and relationship checks. It removes/restores the error association as a
negative control, tests real labels, forward/reverse flows, controller
relations, bounded arrays, successful empty COM properties, password
descriptors and foreign/closed/stale identities. Hidden runs preserve actual
provider availability without showing the window or substituting engine AX.

The 0.1.14 release candidate also repeated three NVDA groups on its newly
compiled production application without `native-agent`: field label/help and
landmarks, physical Tab/Enter skip navigation, and physical typing/Enter to
the real Rust handler. Its actual status node and Speech Viewer both showed
`Hello, Reader fourteen!`. This EXE SHA-256 was
`10fd36f53e365c5760051ec60dc6f86ce234b0e0e3b65a6b3c627d974ad5e872`.
Its independent isolation run rejected development activation and completed
with normal exit. This version-specific production evidence is separate from
the earlier Narrator baseline and the error-description regression above.

## Remaining acceptance

The 0.1.15 production application repeated three actual NVDA 2026.2 groups:
persistent label/help and landmarks, physical Shift+Tab/Enter skip navigation,
and physical typing/Enter to the real Rust handler. Application UIA exposed
`Hello, Reader fifteen!`, and Speech Viewer emitted `Calling Rust...` followed
by that result. The generated EXE SHA-256 was
`0e829aeb62f209a0118245a079d430b5f120f0d8b7e47dc0472d56140452a380`.
Its own isolation run had no debug markers, no session/debug navigation and
normal exit. The default and native fixture production EXEs independently
passed the same isolation checks.

This version repeats 213 native check groups, including 20 invocation-deadline
groups in visible/hidden windows. Sixteen separate shipped-bridge source checks
cover wait limits, callback/timer cleanup and failed transports. A real
16-second Rust/CLI roundtrip remains responsive while another greeting runs.
These IPC checks complement accessibility tests; they do not expand the
assistive-technology certification scope. Narrator observations above remain
the earlier baseline rather than a new 0.1.15 runtime run.

Speech Viewer and Speech Recap prove emitted utterances, not audible
intelligibility. Braille hardware, voice control, switch access, physical external
file drops, Windows 10, and non-Windows assistive technologies require separate
runtime acceptance. Forced-color and reduced-motion tests use real engine media
emulation; they do not certify every Windows theme or OS setting transition.
OS dialogs beyond the verified message box and WebView2 file picker also need
their own tests. Hidden providers differ from the full engine AX tree.

The native collection fixture retains axe incomplete contrast results and
independent rendered-color measurements; the starter's zero-incomplete result
does not replace those limits. See [the native contract](NATIVE_AGENT.md).
