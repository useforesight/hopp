# Tuple features missing from Hopp

October 2026. Compares Tuple with upstream Hopp (Mac/Windows) and with this Linux client.

## Summary

Hopp covers Tuple's core pairing well: calls, rooms, screen sharing, remote control, drawing and a shared clipboard. Tuple is ahead in five areas: guest and link access, viewer controls for a share, reactions, encryption and admin, and its 2026 AI and agent tools.

Of the 57 Tuple features compared, 40 are missing from Hopp on Mac/Windows and 40 from the Linux client.

## The biggest gaps

- **AI and agents.** This is most of what Tuple added in 2026, and Hopp has none of it:
  - **Capture** records calls on the device and makes a transcript with on-device Whisper.
  - **Call history and screenshots:** past calls and transcripts are searchable, and you can take a screenshot of the shared screen.
  - **CLI and MCP server:** both are aimed at Claude Code and similar agents.
  - **`tuple connect`** starts an agent that follows the call.
- **Outside people.** Tuple has free guest accounts, reusable call links and "knock to join". Every Hopp call or room needs a team member.
- **Encryption and admin.** Tuple encrypts media end to end and sends calls of up to two or three people peer to peer (Tuple's own pages disagree on the cut-off). Hopp's media is decrypted on the LiveKit server, so whoever runs that server could see it. Tuple also has SAML SSO, SCIM and usage reports.
- **Viewer controls for a share.** In Tuple, viewers choose the resolution, ask someone to share, take over sharing and open links on the host. It can also share system audio and hide notifications during a share.
- **Reactions.** Tuple has emoji, GIFs, sound reactions and text annotations.

## Where Hopp is ahead

- **Open source and self-hostable.** Tuple can't be self-hosted.
- **Half the price:** $15 per user per month against Tuple's $30.
- **Linux.** Tuple's Linux app is an alpha. Its documented limits:
  - It can't share a camera.
  - It can't switch audio devices.
  - Nobody can control a Linux machine remotely.
  - It has no Capture.

  This client does the first three, and adds:
  - a viewer that follows the sharer's cursor
  - peek
  - the bar plugin
  - an emergency shortcut to take back control
  - a stop on sharing and remote control when the screen locks
- **Browser.** Hopp's web app can share a screen. Tuple's web client is an opt-in alpha that can only watch.
- **Several controllers.** Upstream Hopp lets more than one person control at once.

## Suggested next steps for the Linux client

1. **Match upstream on drawing.** Fading strokes, the click highlight and a colour per person are all already in Hopp's message format. They're missing only from the Linux host's drawing layer (`src/annotations.rs`).
2. **Speaking indicators and a "you're muted" alert.** The audio levels are already measured; neither is shown.
3. **Triggers.** Run your own scripts on call events. The bar backend already gets every state change, so this is small, and it covers desktop notifications, Slack status and the like.
4. **Missed-call notification** via `notify-send`, with a Call back action.
5. **Push-to-talk** as a Hyprland key bind.
6. **Call stats** in the bar panel or a small window.

Capture, transcripts and an MCP server would be the biggest additions, but they're a project of their own. End-to-end encryption would need changes to the server and both clients.

Other gaps against upstream Hopp: Slack join links (`src/api.rs` rejects them), customisable shortcuts, a camera shortcut, a mic meter, room rename and delete, a resolution setting, a noise-suppression toggle, control of window shares, and IME input.

## Comparison

Yes: has it. Partial: some of it. No: missing. ?: not verified. –: web app, not a client feature.

### Calling and presence

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Free guest accounts for outside collaborators | Yes | No | No | Tuple guests see only the person who invited them. Hopp links need team membership. |
| Reusable personal call link | Yes | Partial | No | Hopp has room links for its web app, team members only. |
| Turn a live 1:1 call into a joinable link | Yes | No | No | New in Tuple, Sep 2026. |
| Personal room with an open/closed door | Yes | No | No | Closed door: visitors ask to be let in. |
| Knock to join a teammate's call | Yes | Partial | Partial | Hopp joins ongoing calls directly. No approval step found. |
| Alert when teammates enter a room | Yes | No | No |  |
| Missed-call notification with Call Back | Yes | No | No |  |
| Decline with a message | Yes | No | No |  |
| Favourite teammates and rooms | Yes | No | No |  |
| Hide who you're pairing with | Yes | No | No | Hopp always shows call peers to the team. |
| Mute mic and camera when the screen locks | Yes | No | Partial | Linux client stops sharing and control on lock. |
| Alone-on-call reminder, hang up on sleep | Yes | No | No |  |

### Screen sharing

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Share up to 5K | Yes | Partial | Partial | Hopp desktop caps at 4K. Linux scales to 4096 px wide. |
| Share part of a screen, or snap to a window | Yes | Partial | Partial | Tuple: macOS only. Hopp window share: macOS only, or view-only on Linux. |
| Ask someone to share, or take over sharing | Yes | No | No |  |
| Viewer chooses resolution; upscaling | Yes | No | No | Hopp sets resolution on the sharer's side only. |
| Zoom and pan a large share | Yes | ? | Yes | Linux viewer adds follow-the-cursor and a minimap. |
| Hide chosen apps from the share (App Veil) | Yes | Partial | No | Hopp: macOS only. |
| Hide notifications while sharing | Yes | No | No |  |
| Share system audio | Yes | No | No | Tuple: macOS, headphones only. New Oct 2026. |
| Send the focused window's title and URL | Yes | No | No | Tuple: macOS. New Sep 2026. |

### Remote control

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Forward ⌘-Tab and system shortcuts | Yes | ? | No | Not found in Hopp code. |
| Open a link in the host's browser | Yes | No | No | Tuple's Send Link. |
| Remove a participant from a call | Yes | No | No |  |
| Choose your cursor colour | Yes | No | No | Hopp assigns colours. |

### Drawing and reactions

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Click highlight on the host's screen | Yes | Yes | Partial | Linux host doesn't draw it. |
| Strokes that fade | Yes | Yes | Partial | Linux host keeps every stroke until cleared. |
| Drawing colour per person | Yes | Yes | Partial | Linux host draws one colour. |
| Text and emoji annotations | Yes | No | No |  |
| Emoji and full-screen reactions | Yes | No | No |  |
| GIFs and sound reactions | Yes | No | No |  |

### Audio and video

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Push-to-talk | Yes | No | No | Tuple: macOS. |
| "You're muted" alert when you speak | Yes | No | No |  |
| Mute others, or ask them to unmute | Yes | No | No |  |
| Speaking indicators | ? | Yes | No | Not documented by Tuple. |
| Noise suppression on/off | Yes | Yes | Partial | Linux: always on, no toggle. |

### Capture and AI

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Record calls locally with an on-device transcript | Yes | No | No | Tuple Capture. Whisper on the device. |
| Search past calls and transcripts | Yes | No | No |  |
| Screenshot of the shared screen | Yes | No | No |  |
| CLI for calls, rooms and contacts | Yes | No | Partial | Linux: a few flags (--peek, --quit, --revoke-control). |
| MCP server for coding agents | Yes | No | No | About 35 tools. Claude Code, Codex and others. |
| Start an agent with call context; in-call cards | Yes | No | No | tuple connect, tuple notifications. |

### Integrations

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Run scripts on call events (Triggers) | Yes | No | No | 16 events, plus a directory of 35 scripts. |
| Google and Apple Calendar call links | Yes | No | No |  |
| Slack slash command | Yes | Yes | No | Linux client rejects Slack join links. |
| Launcher integration | Yes | No | Partial | Tuple: Spotlight, Raycast. Linux: bar plugin and Super+Alt+H. |
| Usage API | Yes | Partial | – | Hopp has an OpenAPI spec but only user tokens. |

### Admin and security

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| End-to-end encryption; peer-to-peer for small calls | Yes | No | No | Hopp media passes decrypted through the LiveKit server. |
| SAML SSO and a required sign-in method | Yes | No | – |  |
| SCIM provisioning | Yes | No | – |  |
| Owner, Manager and User roles | Yes | Partial | – | Hopp has one admin flag. |
| Pairing insights and usage export | Yes | No | – |  |
| Call-link policies | Yes | No | – |  |
| Call history | Yes | No | No |  |

### Everyday use

| Tuple feature | Tuple | Hopp Mac/Win | Hopp Linux | Notes |
|---|:-:|:-:|:-:|---|
| Call stats window | Yes | Yes | No |  |
| Customisable global shortcuts | Yes | Yes | No |  |
| Snooze notifications | Yes | No | No |  |
## Limits of this comparison

- **Sources and dates.** Tuple facts come from tuple.app, docs.tuple.app and its changelog, as of October 2026. Hopp facts come from the code at upstream commit 0350ced plus this Linux client.
- **"?" cells.** These couldn't be confirmed from code or docs. They include zoom on the Mac/Windows app and whether Hopp passes system shortcuts like ⌘-Tab through.
- **Not found in Tuple's sources:** frame rate, pausing a share, do-not-disturb, speaking indicators, an emergency revoke, text chat, file transfer, Outlook, IDE plugins, Zapier or webhooks, audit logs, data residency and self-hosting. Tuple states it holds neither SOC 2 nor ISO 27001.
- **Tuple's docs contradict themselves in places:** Windows "beta" against "supported", browser guests against an opt-in web alpha, SSO plan names, and "never sent to servers" against relayed larger calls. These were left out of the table.
- **Untested in the Linux client:** calls with Mac/Windows Hopp users, the shared clipboard, and the viewer's follow mode on a live share.
- **Upstream Hopp bugs seen along the way:**
  - The 10-person cap isn't enforced; nothing in the code limits it.
  - The code plays `call-rejected.mp3`, which doesn't exist.
  - The upstream Linux core stubs out remote input, and `use_av1` probably breaks its Linux build.

## Sources

- https://tuple.app
- https://tuple.app/pricing
- https://tuple.app/security
- https://tuple.app/triggers/directory
- https://docs.tuple.app
- https://docs.tuple.app/getting-started/tuple-for-linux
- https://docs.tuple.app/cli/mcp
- https://docs.tuple.app/changelog
