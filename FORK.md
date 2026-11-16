# Fork deviations

This fork of [OpenDeck](https://github.com/nekename/OpenDeck) carries local
changes on `main`. Keep this file up to date so a rebase onto a new upstream
release is mechanical.

## 1. Allow any plugin to send `switchProfile`

**File:** `src-tauri/src/events/inbound/mod.rs` (one line, plus a comment)

Upstream drops the `switchProfile` event from any plugin other than the bundled
Starter Pack. We remove `InboundEventType::SwitchProfile(_)` from that gate, so
any plugin may change the selected profile. `DeviceBrightness` is left gated.

**Why:** a key holds exactly one action instance, and a plugin may only set the
image of its own instances. With `switchProfile` restricted, a key can either
show live state painted by a plugin *or* open a profile page — never both. This
lets one key do both, which is what the Tailscale plugin's status keys need.

**Upstream position:** deliberate, not an oversight. See
[PR #409](https://github.com/nekename/OpenDeck/pull/409), closed unmerged:

> This event is indeed only for plugin-supplied profiles. OpenDeck has its own
> internal event that is currently limited to use by the Starter Pack only in
> order to prevent third-party plugins causing unexpected behaviour. I don't
> believe that switching between the user's own profiles without a clear
> indicator of what caused the switch will ever be a good UX.

Elgato's own `switchToProfile` is for plugin-supplied profiles, which OpenDeck
does not support; this deviation reuses OpenDeck's *internal* event instead, so
it does not change the meaning of the Elgato SDK command.

**On rebase:** search for `FORK DEVIATION` in the tree. If upstream restructures
the authorisation block in `process_incoming_message`, re-apply by removing
`SwitchProfile` from whatever allowlist guards it.

## Building

See `docker/` — the build runs in a container so the host needs no Rust, Deno or
webkit development packages.

On Windows: `scripts/build-windows.ps1` (Rust + MSVC build tools + Deno, built
natively), `-Install` to install the NSIS bundle silently without a desktop
shortcut and restart OpenDeck into the tray.

Then install over the stock package:

```sh
sudo apt install --reinstall ./src-tauri/target/release/bundle/deb/opendeck_*_amd64.deb
```

`--reinstall` is needed because the fork keeps upstream's version number, so apt
otherwise considers it already installed. If you would rather tell them apart in
`dpkg -l`, bump `version` in `src-tauri/Cargo.toml` before building.

Nothing else on the system points at an OpenDeck apt repository, so an
`apt upgrade` will not quietly replace this build with an upstream one.

## 2. Encode device images at maximum JPEG quality

**Files:** `src-tauri/src/events/outbound/devices.rs`, `src/lib/rendererHelper.ts`

Encoder (dial strip) images were encoded with the `image` crate's default JPEG
quality of **75**, and key images with the browser's `toDataURL` default. Both
are then re-encoded by mirajazz at quality 90 on the way to the panel, so every
image was compressed twice and the strip's first pass was poor — clearly visible
as ringing around the numerals on the 200x100 strip canvas.

Both are now maximum quality, leaving mirajazz's 90 as the only meaningful loss.

**Why not lossless:** the device plugin rejects anything that is not JPEG
(`if url.mime_type().subtype != "jpeg"` in opendeck-akp05's `handle_set_image`).
mirajazz does implement `ImageMode::BMP`, so a lossless path is conceivable, but
it would mean also forking opendeck-akp05 and it is unknown whether the N4
firmware accepts BMP for protocol version 3.

## 3. Render at the device's native resolution

**Files:** `vendor/streamdeck-strip-render/src/render.rs`, `src-tauri/Cargo.toml`
(`[patch]`), `src-tauri/src/encoder_layouts.rs`, `src/components/DeviceView.svelte`

Both the dial strip and the keys were being resampled by a non-integer factor on
the way to the panel, which beats against the pixel grid and makes curves and
diagonals visibly wavy:

| surface | canvas | panel | factor |
| --- | --- | --- | --- |
| dial strip | 200x100 (Elgato SD+ segment) | 176x112 | 0.88 x / 1.12 y |
| keys | 144x144 | 112x112 | 0.78 |

mirajazz gets there with `resize_exact`, so matching each canvas to the panel
makes that call a no-op and removes the resample entirely.

- The strip canvas lives in `streamdeck-strip-render`, a git dependency, so it is
  **vendored** at the exact locked rev (23fa1399) with `CANVAS_W`/`CANVAS_H`
  changed to 176x112, wired in via a Cargo `[patch]`. It supersamples 4x
  internally, so it now renders 704x448 and lands natively.
- `encoder_layouts.rs` has two 200x100 fallbacks, updated to match.
- Keys: `DeviceView.svelte` passes 112 for plugin devices. Elgato keeps 144,
  which is a clean 2x downscale to its 72px keys -- 112 would make it worse.

**Consequences:** Elgato's built-in `$A0`/`$B1` encoder layouts assume a 200x100
canvas and will be geometrically wrong on this build. Irrelevant here (no Elgato
hardware, and the dial plugins use a custom full-canvas layout), but it is the
reason this cannot go upstream as-is -- a proper fix would make the canvas
per-device rather than a constant.

## 4. Log encoder layout failures instead of discarding them

**Files:** `src-tauri/src/store/profiles.rs`, `src-tauri/src/events/frontend/instances.rs`

Upstream calls `initialise_encoder_layout` with `let _ =`, so a failure is
silently dropped. It leaves `layout_parsed` as `None`, and every later
`setFeedback` then fails with only "Layout is not loaded", with no hint as to
why. Both call sites now log the error as a warning.

**Upstream position:** not proposed; a candidate for a PR, since it changes no
behaviour.

## 5. Start hidden on Linux instead of flashing the window

**Files:** `src-tauri/tauri.linux.conf.json`, `src-tauri/src/main.rs`

Upstream creates the main window visible on Linux and, with `--hide`, hides it
again in `setup`. Between the two the window is mapped, so every start flashes
an OpenDeck entry in the taskbar. That is every resume here, because
`opendeck-power` stops OpenDeck across suspend. Windows already avoids this:
`tauri.windows.conf.json` creates the window with `"visible": false` and `setup`
shows it only without `--hide`. Linux now does the same. The window block in
`tauri.linux.conf.json` is the full copy from `tauri.conf.json`, because the
platform config is a JSON merge patch and arrays are replaced, not merged.

**Upstream position:** not proposed; a candidate for a PR, since Windows
already behaves this way.

**On rebase:** if `tauri.conf.json`'s window block changes upstream, copy the
change into `tauri.linux.conf.json` too.

## 6. Forward plugin-rendered JPEG keys to the device untouched

**File:** `src/lib/rendererHelper.ts` (end of `renderImage`)

Key images go plugin -> frontend canvas (a browser rasterising the SVG and
encoding a JPEG) -> device. The `dev.ruben.*` plugins now render their keys
themselves with resvg (the same renderer and bundled Noto Sans as the strip
renderer) at the panel's native 112x112 and send a JPEG data URL, measuring the
wire size with the device plugin's own encoder. When a key's image is such a
JPEG and nothing would be drawn over it -- no title text, ok/alert overlay,
press shrink, rotation, scaling or background colour -- the frontend forwards
the plugin's bytes to `update_image` instead of the canvas's.

**Why:** a browser to draw a key is overkill, its fonts differ from the strip
renderer's, and nothing on that path knew how large the result would be --
which matters on a panel whose firmware truncates (and can hang on) images over
roughly 10 KB. The canvas still draws the key for OpenDeck's own window.

**Upstream position:** not proposed.

**On rebase:** re-apply at the `update_image` call at the end of `renderImage`.

## 7. Send plugin-rendered JPEG keys to the device from the backend

**Files:** `src-tauri/src/events/frontend/instances.rs` (`update_state`,
`passthrough_image`), `src-tauri/src/events/outbound/devices.rs` (`update_image`)

Even with deviation 6, every key image reached the device only after the
frontend had rendered the key: plugin -> backend -> `update_state` event ->
WebView -> `invoke("update_image")` -> backend -> device plugin. So the WebView
was on the path of every key update. Now `update_state` itself forwards a key
whose image qualifies for deviation 6's pass-through (a JPEG data URL with no
title, scaling, background colour or device rotation) to the device, for the
selected profile only, as the frontend's command does. The frontend still sends
the same bytes a moment later; `outbound::devices::update_image` drops a key
image identical to the one sent to that key within the last 2 s, so the panel
is written once. Different bytes (the press-shrink effect) and later resends
(reconnect, wake) always go through.

Dial strips likewise: the frontend used to render each encoder and call
`update_image`, which composes the strip from the layout and feedback in Rust.
While there is no window (deviation 8), `update_state` makes that call itself
(with the window open the frontend still does, so strips are not sent twice).

**Why:** keys keep updating whatever the WebView is doing, which is the
precondition for suspending or closing it while OpenDeck sits in the tray --
it is ~390 MB and most of OpenDeck's idle CPU.

**Upstream position:** not proposed.

**On rebase:** re-apply in `update_state` and at the top of the outbound
`update_image`; keep `passthrough_image` in step with deviation 6's condition.

## 8. Destroy the WebView while OpenDeck is in the tray (Windows, Linux)

**Files:** `src-tauri/src/main.rs` (`show_window`, `hide_window`, setup, tray,
`RunEvent::ExitRequested`), `application_watcher.rs`,
`events/frontend/{instances,mod,profiles}.rs`, `events/inbound/misc.rs`

Hiding the window now destroys it on Windows and Linux, and starting with
`--hide` never keeps one; showing (tray click, Show, a second launch) builds it again from the
app config. Its WebView2 was ~390 MB and most of OpenDeck's idle CPU while
nobody looked at it. Made possible by deviation 7 (keys reach the device from
the backend). Everything that assumed the window exists now copes without it:

- every `get_webview_window("main").unwrap()` is an `if let`;
- a plugin's `switchProfile` and the application watcher's profile switches go
  to the frontend when there is a window (unchanged) and otherwise call
  `set_selected_profile` directly -- what the frontend would have invoked;
- `RunEvent::ExitRequested` without an exit code (the last window going) is
  prevented; Quit and Restart carry a code and still exit.

**Lost while hidden:** `showAlert`/`showOk` overlays (drawn by the frontend),
plugin brightness requests (applied via the settings page), and keys whose
image the backend does not forward (titles, static icons) -- none of which the
dev.ruben.* layout uses. On Linux that frees WebKitGTK's web and network
processes (~270 MB together, measured 2026-11-16 with the tray idle); the
dev.ruben.* plugins draw their own alert face instead of relying on
`showAlert`, so a refused press still shows. Also needed without a window:
the frontend used to send the device brightness again when it loaded, and the
MSD-Pro ignores the one sent at registration after akp05's shutdown command
(every image arrives, the panel stays dark). So `register_device` sends it
again 3 s later and redraws the selected profile from the backend
(`events/inbound/devices.rs`).

**Upstream position:** not proposed.

**On rebase:** grep for `FORK DEVIATION 8`; check any new
`get_webview_window("main")` use for the absent-window case.
