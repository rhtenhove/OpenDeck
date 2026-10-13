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
