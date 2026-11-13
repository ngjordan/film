# Film shell integration report

## Delivered

`crates/film-shell` is package `iridium-film-shell`, lib import `iridium_film_shell`.
The workspace's `crates/*` glob includes it. Its layer is L6
(`xtask/src/main.rs:76`, `docs/architecture.md:25`). No binary-to-shell dependency
was added, so no same-layer exception is needed. The standalone binary continues
to construct `FilmcraftApp`; its logic/UI/exit implementations now share the
host-callable lifecycle helpers.

All changes are confined to this repository, uncommitted. The Office repository
was read for the contract, not modified. Suite adapter/routing/installers/workflow
changes and a multi-module suite rendering test remain the parent's work.

## Exact public API

All signatures are in `crates/film-shell/src/lib.rs`:

```rust
pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self; // :47
pub fn new_for_host(ctx: &egui::Context, _storage: Option<&dyn eframe::Storage>, launch: Launch) -> Self; // :56
pub fn logic(&mut self, ctx: &egui::Context); // :94
pub fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput); // :98
pub fn ui(&mut self, ui: &mut egui::Ui); // :102
pub fn backdrop(&self) -> [f32; 4]; // :112
pub fn open_path(&mut self, path: &str); // :116
pub fn save_state(&mut self); // :138
pub fn can_close(&self) -> bool; // :142
pub fn zoom(&self) -> f32; // :146
pub fn fonts(&self) -> egui::FontDefinitions; // :150
pub fn install_native_menu(&mut self); // :154
```

The suite's Module methods forward one-to-one; `new`/`new_for_host` and
`backdrop` are constructor/look support rather than Module trait methods.
`Launch` (:30) has `files: Vec<String>`, `data_dir: Option<PathBuf>`, `demo: bool`,
and `control: Option<Receiver<filmcraft_ui_egui::ControlRequest>>`; it implements
Default. Settings live in the app profile, not eframe Storage.

`new` additionally installs an available wgpu render state. `new_for_host` uses
CPU composition and needs no CreationContext or window. Audio, clipboard,
file-dialog, system-language and appearance hooks are installed; existing audio,
menu and platform implementations are reused by path rather than rewritten.

## Fonts, style and lifecycle

- `FilmcraftApp::install_look`, `crates/ui-egui/src/app.rs:206`, is the existing
  lazy theme routine made reusable. Both normal logic (:324) and the shell's
  constructor (:81) call it. The constructor also applies saved appearance and
  scale preferences (:82-86) before host look capture.
- `theme::font_definitions`, `crates/ui-egui/src/theme.rs:426`, builds the actual
  Latin/custom families and CJK/RTL fallbacks. It also includes the system
  Japanese fallback when craft-fonts does not provide Japanese, so reporting
  fonts cannot lose that queued addition. The existing `theme::install`
  signature is unchanged (:420). The constructor never reads Context fonts.
- Font installation is construction-only/change-gated. Theme visuals are
  updated through `app/prefs.rs:5` and its preference-change/system-theme checks.
  UI scale is change-gated at `app/prefs.rs:36`; Reset Zoom also sets it (:53).
  The `rtl.rs:236` set_fonts call is inside a test, not production.
- If lazy construction occurs *inside* an egui pass, set_fonts takes effect next
  pass; shell UI (:103-107) waits rather than drawing with an absent custom
  family. Tests cover construction both before the first pass and inside a pass.
- `logic_for_host` (:521) and `ui_for_host` (:541) preserve the existing IME
  lifecycle without constructing a fake eframe Frame. Context IME callbacks are
  installed idempotently by the shell (:57).
- `save_state_for_host` (:527) cancels captures, ends IME viewport state, and
  flushes/stops persistence. Unlike standalone on_exit it does not invoke the
  process updater, and neither shell save nor quit calls process::exit.

## Actual opened/imported file types

Project: `fcproj` (shell :117; native project picker at
`apps/filmcraft/src/main.rs:459-465`). Project opens use the existing
`ask_before_open` dialog so a routed file cannot silently replace dirty work.
Other paths use the engine's existing `file.import` command, which performs
media, caption, interchange and preset detection
(`crates/engine/src/commands.rs:1804-1875`).

`crates/media/src/lib.rs:267-271` defines:

- Stills: `png jpg jpeg gif webp tif tiff bmp tga dpx exr hdr dng psd`.
- Audio: `wav wave bwf aif aiff mp3 mp2 m4a aac flac ogg opus`.
- Video: `mp4 m4v mov mkv webm avi mxf mts m2ts m2t ts mpg mpeg vob mod tod m2v m1v mpv 3gp y4m apv`.

Additional imports:

- Interchange: `edl xml fcpxml otio aaf omf omfi`
  (`crates/engine/src/interchange.rs:12`).
- Captions: `srt vtt scc webvtt mcc stl ttml dfxp xml`
  (`crates/engine/src/captions.rs:102-106`).
- Effect presets: `prfpset`; FilmCraft preset JSON is also recognized by content
  (`crates/engine/src/presets.rs:497,510-547`). Do not route arbitrary JSON to Film.

The ordinary Import picker lists a narrower subset
(`crates/ui-egui/src/app/files.rs:14-18`); its caption-specific picker is at :82.
These are supported imports, not a request to steal shared image/audio/XML file
associations from existing suite modules. `.fcgt` templates and other dedicated
settings/preset pickers are not claimed as generic open-path associations.

## Unsaved changes

`Session::is_dirty` is `revision != saved_revision`
(`crates/engine/src/lib.rs:829-830`). `can_close` delegates to
`FilmcraftApp::can_close_for_host` (`crates/ui-egui/src/app.rs:536`), using that
existing state plus the existing user-confirmed-discard flag `quit_confirmed`.
The existing quit prompt checks exactly these conditions at
`crates/ui-egui/src/panels/clip_dialogs.rs:168-175`; no parallel dirty flag was added.
`Session::shutdown` (:733) flushes recovery; dirty work remains recoverable and
is not implicitly saved over the project file. The host must negotiate close
before calling save_state.

## Native menu and process ownership

The actual native menu is **not** in ui-egui; it is
`apps/filmcraft/src/native_menu.rs:42`:

```rust
pub fn install(app: &FilmcraftApp, ctx: egui::Context)
    -> (Receiver<String>, MenuUpdater, NativeTextMenu);
```

The shell's mutating wrapper installs its command inbox, menu-update hook and
native color picker and hides the in-window menu on macOS. Elsewhere it is a
no-op. Replaced the original leaked bar/receiver thread (:158-167) with a
replaceable `MenuEvent::set_event_handler`, as the suite's existing menu shells
use. The NativeTextMenu closure retains the bar; repeated app switches no longer
spawn competing global receiver threads or leak a bar per switch. This macOS
path was source-reviewed but not compiled here.

The control server is also actually in **apps/filmcraft**, not ui-egui:
`apps/filmcraft/src/control_server.rs:67`. It binds 127.0.0.1 at the requested
port (:81), requires a configured 64-hex-character token (:75), authenticates
every JSON-lines request with constant-time comparison (:195-199), limits lines
to 4 MiB and connections to 16 (:24-26), uses per-connection threads, wakes egui
and queues ControlRequests to the UI, waiting up to 60 seconds for replies
(:157-160). Startup requests are rejected until a ready flag is set. Invalid
requests close the connection. Duplicating it inside modules would contend for
the same process port, keep detached accept threads alive, and create multiple
process-level authenticated endpoints. The shell never reads token/port env
variables and never starts a listener. An authenticated host may supply its own
request receiver in Launch. Logging, panic hooks, graphics backend selection,
hardware-decoder/export-renderer registration, update ownership and process
startup remain the host binary's responsibility.

## Naming and compatibility

Changed the new shell package/name/docs to Film / iridium-film-shell. Switched
**all 14** egui-family patch entries actually present in Cargo.toml:137-150 to
ngjordan/egui at faf1ced22250c509636bff2578731a0be86a69ac; also switched winit's
source URL (:151) to ngjordan at the unchanged pinned revision so Cargo uses the
same source as the fork. Cargo.lock was updated by Cargo.

No blanket rebrand was attempted. Existing app id, binaries, internal crates,
profile directories, project/magic/protocol/MIME/cache/codec/container strings,
update identity and upstream assets/attribution are intentionally unchanged.
There is no new profile-writing scheme, so no legacy read migration is needed.
The app is still visibly upstream-branded; a safe broader rebrand (including
removal/replacement of upstream trademark assets before distribution) remains.

## Verification (Linux)

All Cargo commands used CARGO_BUILD_JOBS=4, CRAFT_FONTS_DIR=/tmp/craft-fonts,
CRAFT_I18N_DIR=/tmp/craft-i18n, the supplied ALSA pkgconfig/library paths, and
umask 022. Both resource paths were verified as actual git checkouts; the font
manifest exists. No resource checkout was created or modified.

| Exact command | Actual result |
|---|---|
| `cargo check -p iridium-film-shell` | PASS: Finished dev profile in 2m 17s |
| `cargo build -p iridium-film-shell` | PASS: Finished dev profile in 6m 21s |
| `cargo test -p iridium-film-shell -j 1` (final) | PASS: 23 passed; 0 failed; 0 ignored; doc-tests 0 |
| `cargo check -p iridium-film-shell -p filmcraft` | PASS: Finished dev profile in 14.80s |
| `cargo build -p iridium-film-shell` (final) | PASS: Finished dev profile in 0.33s |
| `cargo clippy -p iridium-film-shell --all-targets -- -D warnings` | PASS: Finished dev profile in 27.41s |
| `cargo xtask layers` | PASS: layers: ok |
| `cargo xtask assets` | PASS: assets: ok (72 assets attributed) |
| `cargo fmt --all -- --check` (final) | PASS, no output |
| `git diff --check` | PASS, no output |
| `cargo check -p iridium-film-shell --all-targets --target x86_64-pc-windows-msvc` | BLOCKED in ring's build script: failed to find tool lib.exe; no MSVC cross C toolchain installed |

Initial test runs exposed a missing inherited test-only glob dependency and the
fork's requirement to explicitly clear unapplied TexturesDelta in headless tests;
both were fixed before the passing run. Initial formatting check required
formatting the new files; the final formatting check passed. Cargo emits unused
patch warnings for optional egui packages; clippy nevertheless passed. ALSA
reported no sound card in headless tests, so real audio output/input was not
verified. Tests exercise real egui painting, fonts and dirty-open protection,
not a real desktop window, media export or hardware playback.

The full repository-wide `cargo xtask ci` was NOT run. macOS native menu/color
UI and Windows-specific runtime/FFI paths are NOT verified. The Windows cross
check was attempted, not passed. No suite-wide compilation or GUI integration
was performed because modifying/building the Office repository is outside this
worker's scope. Logs are retained under ignored `target/film-shell-verification/`.
Final observed free space: 166 GiB (df -h /).
