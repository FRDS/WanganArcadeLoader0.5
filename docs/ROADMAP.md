# WanganArcadeLoader 0.5 — Windows port roadmap

Where the Windows port stands, what was decided, and the research for the
phases not yet started. For how the crate is put together, see
[CLAUDE.md](../CLAUDE.md); for building and running, see that file and
[dist-windows/README.md](../dist-windows/README.md).

The loader builds from **one crate** into `libwal_3dxp.so` (Linux, i586) and
`wal_3dxp.dll` (Windows, i686). On Windows the game's Linux `main` runs under
[LINE](https://github.com/axylol/line), a Windows ELF loader, and this DLL is
LINE's plugin.

## Status

| Phase | State |
|---|---|
| P0 sync with upstream (30 commits), review fixes | done |
| P1 platform layer, Windows boot | done, proven on hardware |
| P2 window + OpenGL | **done, rendering confirmed on hardware** |
| P3 input (keyboard, pads, wheels) | keyboard and pad confirmed; wheels/pedals still untested |
| P4 custom resolution + file/TTY redirects | done; resolution proven on hardware at 1440×1080 windowed and borderless, `file_redirect` and `ignore_custom_ioctls` both on Windows now |
| Widescreen placement (`widescreen_anchor`) | in-race HUD done and confirmed on hardware; menus and full-width art deliberately excluded |
| P5 custom BGM | not started |
| P6 Gemballa unlock | not started, fully researched below |
| P7 network / localhost split-screen | not started |

Proven on hardware (Export 3DX+ dump): boots through LINE with 47/47 hooks
resolved, reaches `admCreateWindowi`, renders menus and a lit 3D world, and
plays a race. Stack alignment, LINE's hook table, symbol lookup and Rust std
all work. Confirmed on **Intel Iris Xe** and **Intel UHD**, the latter on a
hybrid laptop beside an RTX 2070 Max-Q.

Also proven, 2026-10-04/05:

- The bundle runs with nothing fetched by hand — our own `line.exe`,
  `soft_oal.dll` and the Cg runtime.
- The shader capability probe works under LINE. It creates a GL context at
  `init()`, before the game has one, and leaves GLFW for `adm.rs` to
  initialise afterwards.
- **The Cg compiler inside `cg.dll` is equivalent to standalone `cgc`.** All
  38 programs of a clean dump compile byte-identically to cgc 3.1.0013's
  output (615 instruction lines each side); only the `# command line args`
  comment differs. So `shader.rs` needs no toolkit install and gives up
  nothing by not having one.
- **The probe sees the GPU the game renders with, including on a hybrid.**
  The OpenGL ICD is loaded once per process, so the game inherits whatever
  card the probe's context brought in. Confirmed on the Intel UHD + RTX 2070
  Max-Q laptop both ways: with no preference set both reported Intel, and
  with Windows' per-application preference pointed at the discrete card the
  same run logged `driver probe: has all of GL_NV_vertex_program3,
  GL_NV_fragment_program2` beside `GL renderer: NVIDIA GeForce RTX 2070`.
  That is why restoring `*.orig` is automatic again rather than advisory.
  Note the `NvOptimusEnablement` export from our DLL did **not** achieve
  this — the per-application setting did.

- **Audio works**, on every machine and every test since we began shipping
  our own `soft_oal.dll` (OpenAL Soft 1.23.1, cross-built in CI).
- **The game stays up.** It used to end itself inside a couple of minutes;
  with SIGALRM ignored an idle attract loop ran 29 minutes at a flat 60fps and
  was still going when the run was stopped from outside. One other ending
  survives — see the known gaps.

**What one boot run settled, 2026-10-06** (1280x720, `res_log` and `card_log`
on, Iris Xe desktop, 22 seconds of attract at a flat 60fps, no crash):

- **The revision name is `W3P100-2-NA-DAT0-B02`.** Earlier notes here and in
  the tests said `W3P100-2-0-0-A01`, which was a guess that happened to parse.
  The two middle fields are `NA` and `DAT0`, and the dump is revision **B02**,
  not A01. Everything that gates on the revision reads this, so it is worth
  having right.
- **`elfsym` reads 47,937 functions from `main`** in the real process, the same
  count `readelf --dyn-syms` gives. There is no `.symtab` in that dump at all.
- **The card checksum assumption is right.** The installed `card.bin` is 207
  bytes and its stored CRC-16/CCITT-FALSE over bytes `0..0xCD` matches, which
  confirms the layout read out of `clV386CardData` against a card the game
  itself wrote, not just against a synthetic one.
- **The engine asks for the perspective at the *render* aspect, not 4:3.** It
  arrives as `fov 0.7854 aspect 1.7778` at 1280x720, because the engine works
  the aspect out from the `SCREEN_XSIZE`/`SCREEN_YSIZE` that `res.rs` hands
  Lua. So the FOV has to be widened on that basis -- gating it on 4:3 arriving
  means never widening anything at a custom resolution, which is a mistake this
  run caught. 45 degrees becomes 57.8, where the old arithmetic said 60.
- **The fixed-function `glOrtho` carries the same 2D box**, `[0, 640, 480, 0,
  -1, 1]`, and is not corrected. What the engine draws through it is the open
  question; the box being identical means it is not a different projection,
  just a second copy of the same one.
- **No viewport larger than the render size** turned up in attract, so the 2x
  menu-background viewport and the FSAA 2x buffer the C++ loader documents are
  not on this path. Still worth the report for a race.

Still unverified:

- **How the widescreen correction looks.** The arithmetic is unit-tested and
  mockline drives it through the real DLL at 1280x720, but nobody has looked at
  a corrected frame. More pressing now that the FOV is widened by its tangent
  rather than its angle: that is a visible change to the 3D, and the test that
  justifies it proves an invariant rather than an appearance.
- **A race.** Everything ported from tattoohanz's loader is aimed at the path a
  race ends on -- the printer's print commands, the auto-service error latch --
  and all of it is unit-tested and mockline-tested only. The printer used to
  abort the process on five of thirteen commands, so the first full race is
  both the gate on that work and the first time anyone has had a reason to
  expect one to finish.
- **The HUD attribution table**, which `hud.rs` collects but which needs a real
  race: eleven idle minutes of the attract loop produced no `clRaceThread::draw`
  call at all. Play one with `res_log = true` and the table lands in
  `wal_3dxp.log` five seconds in.
- Steering wheels and pedals, and a full race to completion.
- Anything on Linux since the input rewrite — compile-only by decision, so the
  GLFW keyboard switch and the SDL 0.38 bump are untested there.

## Decisions already made

- **Rust, one codebase**, not a C++ fork. The archived C++ port
  (`axylol/line-wal0.5`) is the reference for LINE mechanics only.
- **60fps is not negotiable.** The engine takes one simulation step per frame
  drawn, so the frame rate is the game's speed — holding it below 60 visibly
  slowed the game, and raising it speeds the game up by the same factor.
  Rendering at a monitor's higher refresh would mean synthesising
  intermediate simulation states, which is not reachable from the OpenGL and
  ADM boundary we patch in a closed 2008 engine; presenting duplicate frames
  would be indistinguishable from 60 on a fixed-refresh panel. Anyone wanting
  higher-refresh motion should use an external frame generator with the game
  left at 60. `fps_limit` is the speed control and `limit_fps()` must not be
  weakened.
- **Windows-only testing**; Linux gets CI compile checks.
- **GLFW keyboard on both platforms** (drops `x11`, `device_query`).
- **Distortion is corrected in the projection, not per sprite; placement, if
  it is corrected at all, is a rigid per-widget translation.** The research
  behind both halves is in its own section below. Sharpened 2026-10-06: the
  original sentence said "widescreen", which read as covering placement too,
  and that was never decided — the stretch is one hook, while moving a widget
  cannot be. The route it does rule out is box mode, which fixes the stretch
  by pillarboxing the 2D layer instead of filling the screen.
  This had been deferred because it lived in tattoohanz's fork of the loader
  DLL (`msys-line_wal_3dxp.dll`), which carries dozens of HUD/sprite/anm hooks
  plus force feedback; LINE loads one plugin, so it cannot run alongside ours,
  and his DLL is GPLv3 — as is this project, so asking beat
  reverse-engineering it. **He supplied the source on 2026-10-05**, and what it
  establishes is recorded below.
- **Gemballa patches applied in memory**, never to `main` on disk.
- **ARM is pointless**: the game is 32-bit x86 and our code rewrites its
  machine code, so an ARM build could never load into it. On ARM you run the
  x86 build under emulation (Windows Prism, or Box64/Box32 on Android).

## Widescreen

**What the engine does**, logged on hardware at 1280x960 and 1280x720 with
`res_log`: it asks `igMatrix44f::makeOrthographicProjection` for the same two
boxes at either aspect — `[0, 639, 447, 0, -100, 100]` and
`[0, 640, 480, 0, -1, 1]`, in left/right/bottom/top/near/far order — while the
viewport follows the render size. So the 2D layer is laid out in 640-space and
mapped onto whatever it is drawn into: at 16:9 that is x2.0 against y1.5, a
third too wide. `res.rs::widen` widens the box to the render aspect instead,
centred, which is one hook rather than dozens. Worth knowing for anything else
that reaches for this: only the **six-float** overload is ever called — the
`igPlane`/`igVec3f` one and both `igMatrix44d` ones never fired — and it is
reached a few times a frame, not once a sprite.

**Placement is done for the in-race HUD**, on 2026-10-17, behind
`widescreen_anchor` (off by default). Elements used to keep their 640-space
coordinates, so the HUD sat in the middle 960 of 1280 pixels; `hud.rs` now
moves each widget by the bar, and on hardware the car's dial went from 117
units short of the edge to 18, against 19 measured at 4:3. A race may still
ask for boxes the attract screens do not; `res_log` is how to see them, and
`widen` leaves anything it does not recognise alone.

Three things about it that are not obvious and cost a day to learn:

- **A widget is whatever brackets its quads**, and there are three kinds. An
  anime by its `.anm`, a string by its draw call, and — for the raw entries a
  console submits with neither around them — a run of quads within 8 units of
  each other. Taking a whole console instead gives `-64.00 .. 685.00`, wider
  than the space, because a console is a container for several unrelated
  pieces. This is what finally moved the meter, which per the section below is
  not a `clSpriteAnime` and so had no widget at all.
- **`applyUser` has to be forced before the vertices are read**, not only
  before they are written. `entry()` runs it when the dirty byte at `+0x74`
  is not 1, so an entry the engine finds clean carries whatever was last left
  in its vertices — menu strings read `0.00 .. 0.00` against origins of 48 and
  320. The moving path always forced it, so the two paths measured different
  things and the difference looked like a property of the strings.
- **Menus must not be anchored.** The name entry keyboard is many widgets and
  one layout: `ABCDE` and `FGHIJ` went `-bar`, `LMNO` and `PQRST` went `+bar`,
  `K` straddled the middle, and the row came apart with 213 units of gap
  either side of it. Which is the same conclusion tattoohanz reached — he used
  assets for menus and did not move the in-race HUD — arrived at from the
  other direction. Anchoring them needs layout-level grouping, which is not
  built.

Still open after it: the **`wide`** population — full-width art like
`-480..1120` — which needs stretching rather than shifting, since a shift only
moves the gap to the other edge.

Also open, and measured on 2026-10-06: the engine builds **the same box a
second time through the fixed-function `glOrtho`**, which is reported but not
corrected. The C++ loader widens both and says widening only the GL one has no
effect on the geometry, which reads as the matrix one being what the sprite
pass renders with -- so the question is what else is drawn while that GL state
is current. If it turns out to be the full-width background art, widening it is
not the answer anyway; stretching is (see the skip-list discussion below).

### What tattoohanz's widescreen patcher turned out to be

Worth having in writing, because it answers design questions we would otherwise
have to measure, and because the archive may not be to hand next time. It is
`WMMT3DX+_Widescreen_patcher.zip`: 39 replacement `.anm` layouts, 12 `.png`,
`wm3_installer.exe` (PyInstaller, so a Python extractor reaches it — no
disassembler), and his `msys-line_wal_3dxp.dll`.

- **His transform is a rigid translation, not a scale.** Diffing his layouts
  against the dump's own: 30,572 of 31,171 changed values are exactly ±106
  units — the margin at 16:9 — with the direction chosen per element, left
  going −106 and right +106. That rules out scaling positions about the centre,
  which would spread each element internally: a needle off its dial, gaps in
  per-glyph text.
- **`.anm` animates x per keyframe.** The large files have tens of thousands of
  changed values across several record types (the changed field is +0x1c most
  often, then +0x34, +0x3c, +0x24). So a sprite's position arrives from
  timeline evaluation, and hooking `setPosition` would see almost none of it.
- **He did not move the in-race HUD.** Only 3 of the 39 files are race
  consoles, and `cons_g_last_nokori.anm` is byte-identical to the dump — shipped
  unchanged. `cons_g_speed`, `cons_g_map` and `cons_g_totaltime` are not in the
  archive at all, and the two race files he did touch got irregular hand values
  (+59, −26), not the margin. The other 36 are menus, mostly `topbar`/`bottombar`
  — the full-width bars that a position-spread would leave short, which he
  solved with assets. So whether his DLL re-anchors the race HUD at runtime is
  an open question; his *assets* do not.
- **Licensing decides the route.** His DLL is GPLv3 and so is this project, so
  asking him for source is both cheaper than reverse-engineering and what the
  licence is for; porting GPLv3→GPLv3 with credit is already the plan for P5.
  His `.anm`/`.png` are derivatives of the game's own assets, so study them,
  never ship them.

### What the engine does with sprites, read out of the dump

From the dump's own 20-byte setters with `objdump`, so none of this needs a
hardware run — `main` carries its symbols, 61,109 by `objdump`'s count. (The
`47,937` reported above is the same table counted differently: `readelf -S`
says there is no `.symtab` at all, `.dynsym` holds 61,074 entries, and 47,937
of those are functions with an address, which is what `elfsym.rs` keeps. The
remainder are data, undefined imports and the null entry.)

| class | x | y | scale x |
|---|---|---|---|
| `clSpriteAnime` | `+0x4C` | `+0x50` | `+0x40` |
| `clSpriteFont` | `+0x20` | `+0x24` | — |

The two layouts **differ**, so a per-sprite hook needs the offset per class.
`clSpriteAnime`'s pair confirms the ones in the rescale upstream removed in
`af420be` (`git show 25c5d33:src/res.rs`) — take its offsets, not its
arithmetic: its `/8.0` fudge yields 40px where the geometry says 160.
`clSpriteManager::entry` looked like list plumbing over `clSpriteRootEntry` —
unlinking a pool entry and making a vtable call at `+0x8` — so interception
was going to be at each class's `draw`, patch-call-restore. **The C++ loader
corrects both halves of that**; see below.

`hud.rs` both measures this and does it. **The console wrappers it started as
answered their own question with a no**: five of the twelve consoles never
draw at all, five submit one object a frame at x = 0, and ten of every eleven
sprites arrive under no console — so attribution by bracketing `draw` does not
work, and the positions come from `clSpriteManager::entry` instead, where the
four vertex x floats are final. The wrappers are still installed, because they
are the only thing that says a race is on screen rather than a menu. **It also
found that sprites are not drawn on the main thread**, which the nudge has to
account for — `adm.rs` already marshals sprite-adjacent calls onto the main
thread for exactly this reason.

### What tattoohanz's C++ loader establishes, which is most of the placement work

From the comments in `D:\line-wal0.5-main` (his C++ LINE plugin, GPL-3 as we
are), arrived 2026-10-05. **It is not a git repository, has no history and
there is no second copy**, so this is the record. Each item is his measurement;
what we do with it is ours.

**The transform goes in `entry`/`entryAfter` after all, and our plan was
wrong.** The vtable call at `+0x8` is `applyUser`, slot 2: `entry()` calls it
whenever the dirty flag at `+0x74` is not 1 and *then* copies twelve floats
into the draw buffer, so anything written into the vertices beforehand is lost.
Call `applyUser` yourself, mark the entry clean and offset the finished
vertices, and it works for every entry type. The reason it has to be there
rather than at `clSpriteAnime::draw`: **the race meter is not a
`clSpriteAnime` at all.** `clMeterConsole::drawMeter` submits 17 raw entries a
frame, mixed plain `clSpriteEntry` (absolute coordinates in the vertices) and
`clSpriteMatrixEntry[UV]` (vertices rebuilt by `applyUser` from the 2x3 affine
at `+0x7c`). A transform written against `clSpriteAnime+0x4C` would miss the
meter entirely. Keep `+0x4C`/`+0x20` for `hud.rs`'s measurement; the eventual
move belongs in `entry`, with `clSpriteAnime::draw`/`clSpriteFont::draw`
demoted to marking which widget is being drawn.

`clSpriteRootEntry`, with the provenance he cites:

| offset | what |
|---|---|
| `+0x00` | vptr, compared against `_ZTV<class>+8` to tell the four entry types apart |
| `+0x04` | `imageId` — both `clSpriteEntry::applyUser` @`0x84603ea` and `clSpriteMatrixEntry::applyUser` @`0x846e166` load `[this+4]` and hand it to `clImageResourceManager::getSize()` |
| `+0x08`, `+0x14`, `+0x20`, `+0x2c` | the four finished vertex **x** floats, already in 640x480 space |
| `+0x74` | byte: the vertices are up to date |
| `+0x7c` | 2x3 affine, translation at `+0x8c`/`+0x90` |

**Three lessons about *what* to move, each with the measurement that forced
it.** They are the difference between a HUD that moves and a HUD that comes
apart:

- **Per widget, not per quad.** A string is one quad per glyph, so a per-quad
  decision tears it apart at the dead-zone border.
- **The extent of the whole widget, not of its first quad.** The "till goal"
  readout (`CONS_G_LAST_NOKORI`) keeps digit slots on both sides of the centre
  — caption 227, digits 266/298/330, the "m" at 361 — and which slots exist
  depends on the digit count, so deciding from the first quad flipped the whole
  readout by the bar width as the distance counted down.
- **The union of every frame seen, never shrinking.** The tuning-point progress
  bar starts as a sliver at the left of its frame and grows rightwards. Judged
  on its current extent it sits entirely left of centre, gets pushed out of its
  own frame, and snaps back once it grows past the middle.

**The decision itself**, with the dead zone at 8 units: a widget entirely left
of `320 - 8` moves `-bar`, one entirely right of `320 + 8` moves `+bar`, and
one that spans the middle does not move. His alternative is proportional —
`(centre - 320) * (k - 1)` where `k` is the widened width over 640 — which
keeps composite widgets together (the menu timer's bezel and digits are two
separate groups) at the cost of rigidity.

**The bar is 106.67 units in 640-space at 16:9, by three independent routes**:
his `box_x * 640 / box_w` under box mode, our `widen`'s
`320 * (render_aspect / (640/480) - 1)`, and the 30,572 of 31,171 changed
`.anm` values that are exactly ±106. Three sources, one number, and all three
say rigid translation.

**Naming a widget portably needs the `.anm`, not an id.** Hooking `setAnime`
worked on WMMT3 and 3DX+ but **hung 3DX**, so he recognises an animation from
the data the object already holds: `clSpriteAnime` keeps the loaded `.anm`
between `+0x18` (begin) and `+0x14` (end), and the first dword of that buffer
is the frame count, so the pair (length, frame count) read off disk at startup
identifies it without touching game code. It has to be the file name because
**anime ids are not portable: `menu_g_finish_st_bar.anm` is 178 in WMMT3, 189
in 3DX and 199 in 3DX+.** Nor are image ids enough: the tuning-point bar's fill
is swapped at run time via `setImageReplace`, so the id that reaches a hook is
the replacement's and the one you would list never appears at all.

Walking `sprite_anime_data_file` to resolve a name has three hazards: the
stride is `{path, NAME}` on 3DX/3DX+ but a single pointer on some builds (probe
the second pointer for `"data/"`), entry 0 is a dummy with an empty path on
some builds so it must not terminate the walk, and **reading one entry past the
end crashed WMMT3DX, whose table ends exactly at `0x09522438`** — the end has
to be found by probing readability, not by dereferencing.

Two things no layout edit can move, so they are code:

- The story prologue's "Now at N wins" is a `clSpriteFont` whose position is a
  `setPosition(620.0f, 20.0f)` baked into `clSeqStEnemyInformationThread`'s
  constructor, as an imm32 at ctor`+0x7A7`. Rewriting that immediate is the
  only way, and the text is right-aligned on it, so a longer count grows
  leftwards and cannot run off. 3DX+ only — he gates on an animation only 3DX+
  ships rather than trusting the offset across builds, and refuses the write
  unless the current value is what he expects.
- The outrun FINISH screen draws its rival portraits on a **60-unit grid
  spanning 22..626**, so the row straddles the centre and each part lands in a
  different bucket, tearing the row into clusters a bar apart. The portraits
  have no edge to be pushed to, so the whole widget keeps its authored x. Image
  ids cannot express that: which face is drawn depends on which rivals were
  beaten, and the character-named portraits are outside the numbered
  1721..1754 block.

**Box mode is the conservative alternative we are not taking.** His `hud_43`
shrinks the GL viewport to a centred 4:3 rect for the 2D pass, so the layer is
pillarboxed rather than filling the screen, and paints the bars black — in
menus only, because *during a race the bars have to stay transparent, that is
where the wide 3D lives*. It is an alternative to `res.rs::widen`, not an
addition, and adopting it would reverse the decision above. Worth having
written down because it is the designed answer to the two problems `widen`
creates in menus: full-width background art leaves gaps at the edges (his
`hud_stretch_bg` stretches those quads instead of moving them, since a shift
would just move the gap), and `clMenu3dEnvironment` does not honour the box.
Its companions — clamping oversized render targets and viewports — only make
sense with it.

Two traps that apply to us whatever route we take, both now reported under
`res_log`: `clGraphics::setFsaa()` renders through a **2x supersample buffer**,
and the menu background asks for a viewport at **twice the screen size**, so a
bar width computed from `config.toml` would be wrong for the target actually
bound. Take it from `glGetIntegerv(GL_VIEWPORT)`.

Also his, and not about placement: the lens flare must *not* be boxed or
anchored, because its quads are 2D but positioned from projected 3D NDC; and
the rival marker must be scaled about the centre rather than anchored, because
its sprite space maps to the 4:3 box so it lags the car by 12.5% of its
distance from the centre, and anchoring makes it jump whenever it crosses the
dead zone.

## Windows runtime strategy

Running the game on Windows takes more than our DLL, and the pieces differ in
how far we can go:

| Piece | Status |
|---|---|
| `wal_3dxp.dll` | cross-built from Ubuntu in ~15 s, imports only stock Windows DLLs |
| `line.exe` + `msys-2.0.dll`, `msys-gcc_s-1.dll`, `msys-stdc++-6.dll` | built by the `line` CI job, bundled |
| `soft_oal.dll` | cross-built with mingw-w64 in the `windows` job, bundled |
| `cg.dll`, `cgGL.dll` | extracted from the Cg Toolkit 3.1 installer, bundled unmodified |

**A release is now self-contained**: a user supplies only their own dump.

Two things decided along the way, so they are not relitigated:

- **Precompiled "universal" shaders cannot replace Cg.** The game `dlopen`s
  `libCg.so` *and* `libCgGL.so` and calls `cgCreateProgram`,
  `cgGLLoadProgram` and `cgGLGetLatestProfile` itself; `cgGL` contains no
  compiler and our loader never loads it. The runtime is needed to load a
  shader in *any* format. Separately, shipping converted `.fp`/`.vp` would
  mean redistributing derivatives of the game's own `.cg` sources, which is
  the line this project holds everywhere else.
- **Cg may be redistributed.** An earlier note here claimed otherwise; that
  was wrong. NVIDIA's licence: "The SOFTWARE may be redistributed providing
  that distributed Cg compiler and runtime binaries are unmodified, except
  for decompression and compression." Debian ships it in `non-free` on the
  same basis. Consequence to keep in mind: the release is no longer
  all-free-software, and the Cg binaries must never be stripped.

OpenAL Soft was likewise not a new dependency when the Windows build picked it
up:
upstream's `build` job has compiled it from source and shipped it as
`libopenal.so` since before this fork, and the archived C++ LINE port already
opened `soft_oal.dll` by that name (`al.cpp`, `dlopen`). Windows was simply
missing the build step. Note the licence: OpenAL Soft is **LGPL v2** (parts
BSD-3-Clause), so the binary ships with `licenses/openal-soft/` — upstream's
Linux artifact still has no licence text for it.

**LINE cannot be cross-compiled.** It is a Cygwin program: `pei-i386`, imports
`msys-2.0.dll`, calls `cygwin_attach_dll`/`cygwin_internal`/`cygwin_premain`.
mingw-w64 cannot produce that, so it needs 32-bit MSYS2 on a Windows host —
which is why the `line` job is the only thing in this repo on a Windows
runner, and why `msys2/setup-msys2` is no help (MSYS2 dropped i686 in 2020).
Everything else we ship cross-builds from Linux.

That job exists because making users build LINE themselves was our single
biggest setup failure: a 64-bit MSYS2 build yields `0xc000007b` with
`msys-gcc_s-1.dll` vs `msys-gcc_s-seh-1.dll` as the only tell.

It also unblocks **patching LINE**, which is the larger prize. Three of its
limits are already in our way and none can be touched while LINE is a binary
someone else built: `fopen64` that MinHook refuses to hook (see P4 below), the
whitelist confining the game to its own directory, and `fixPathIfNeeded`
handling only `/tmp`. Building it ourselves from a pinned commit is the
precondition; forking it is not decided and should not be done casually —
the seam at `platform/windows/line.rs` is what keeps LINE replaceable.

## Known gaps, in the order I would take them

Everything here is understood and deliberately left. None of it blocks a
release; the first two are the ones with user-visible consequences.

- **The game renders into the default framebuffer, not our FBO.** That is why
  the render size cannot exceed the window: the engine rasterises into a
  `CONFIG`-sized viewport that a smaller drawable has already clipped, and
  `adm_swap_buffers` only captures what survived. Giving the FBO a depth
  attachment and pointing the engine's framebuffer binding at it would make
  the render size independent of the window in every mode, and would delete
  the `default → FBO → default` capture loop along with a class of bugs. It
  is also a rewrite of the one path proven on three machines, and the FBO has
  no depth or stencil today, so it needs care rather than enthusiasm. Until
  then: fullscreen for any render size that does not fit the window.
- **`fopen64` cannot be hooked** (MinHook declines LINE's stub at `0x40ef00`),
  so some file activity cannot be observed or redirected. This is not
  cosmetic: the
  engine reads `data/shader/*` through it, so the one measurement that would
  settle when the engine loads shaders relative to window creation is the one
  we cannot take.
- **The game can still die of SIGSEGV inside `msys-2.0.dll`**, at
  `+0x14cf0d` reading `0x9b0c0342`, with byte-identical registers across runs
  and an `esp` that is not a user-mode stack pointer. It lands immediately
  after the console prints `seq().getThread().cancel()`, and Cygwin's own stack
  trace comes back empty. Windows exit code `0x8B00` -- SIGSEGV with a core --
  and a `line.exe.stackdump` beside the game. The mechanism that fits: the game
  imports `pthread_cancel@GLIBC_2.0`, which LINE does **not** stub, so it is
  Cygwin's own; but LINE *does* stub `__pthread_register_cancel`,
  `__pthread_unregister_cancel` and `__pthread_unwind_next` as empty no-ops, so
  the glibc cleanup frames the game pushes are never registered and Cygwin
  unwinds a thread that has none. Reproduces on a loader build from before any
  of this work, so it is not ours. Fixing it means patching LINE -- which the
  `line` CI job now makes possible.
  **It is not the same bug as the SIGALRM one, and ignoring SIGALRM does not
  fix it.** It recurred with the alarm fix active -- same address, same
  registers, a stackdump, eleven minutes into an idle run. An earlier
  29-minute run that saw neither ending was luck rather than evidence, so the
  two are independent and only the quiet one is fixed.
  Now that `elfsym.rs` exists, the discriminating question is answerable from
  the next occurrence: does the symbolised stack scan show frames in
  `msys-gcc_s-1.dll` or the game's own libstdc++, or only in `msys-2.0.dll`?
  The first would tie it to the unwinding gap below; the second leaves it where
  it is, a LINE bug.
- **Nothing resolves the `_Unwind_*` family, and we do not know what that
  costs.** tattoohanz's loader binds all fifteen names through LINE's
  `OnResolveSymbol`, on the premise that LINE's `msys-gcc_s` path does not
  resolve them, so C++ exception handling is dead and any throw crashes -- he
  names movie playback, skip and end-of-stream. We implement no
  `OnResolveSymbol` at all, so whatever LINE does with those imports is what
  happens. Three things to settle before copying anything, cheapest first:
  the dump's `main` **does** carry `.eh_frame` (`0x4ed8` bytes) and
  `.eh_frame_hdr` (`0x10c4`), measured with `readelf -S`, so there is unwind
  information to find; an observer-only `OnResolveSymbol` that logs every name
  and resolves none would say which imports LINE actually fails on, at no risk;
  and `_Unwind_Backtrace` called from inside an existing hook would say whether
  a DWARF unwinder can find FDEs for ELF-mapped frames at all. The last is the
  one that decides it: on glibc `_Unwind_Find_FDE` locates `.eh_frame` through
  `dl_iterate_phdr`, which does not exist here, so both mingw's libgcc and
  Cygwin's fall back to a registry that nothing has registered the game in.
  **Binding the family without registering the frames would change the failure
  mode and not the outcome** -- and `0x4ed8` of `.eh_frame` in a 25 MB binary
  says most of the game has no unwind tables anyway.
- **The game's `stdin`/`stdout`/`stderr` may be null, and his loader says that
  is fatal.** Same mechanism, same callback: he binds `_IO_2_1_std*_` and
  `std*` to msys's real `FILE` structs, reporting that they otherwise stay null
  and the game hands a null `FILE*` to msys's `fwrite` and dies inside
  `msys-2.0.dll` -- he names create-card. **His fix must not be copied
  literally**: he builds with the MSYS toolchain so his C runtime's `stdout`
  *is* msys's, while ours is msvcrt's, and handing the game an msvcrt `FILE*`
  that it passes to msys's `fwrite` is worse than handing it null. The source
  has to be `line::resolve_stub("stdout")`, which
  `unbuffer_msys_stdout()` already uses. Whether we have the problem at all is
  one log line away: that function returns silently when the stub's `FILE*` is
  null, so nobody has looked.
- **`limit_fps()` busy-spins the last 2ms of every frame**, about 12% of one
  core continuously. Harmless on a desktop, less so on a Max-Q laptop. A
  swap-interval divisor would remove it on exact-multiple refresh rates, but
  the limiter has to stay as a backstop regardless, so the prize is small and
  the code is the one thing holding game speed correct.
- **Untested:** steering wheels and pedals, a full race to completion, and
  anything on Linux since the input rewrite.

## The test rig, as of this work

Two machines, and confusing them wastes runs.

- **Laptop** — Intel UHD + RTX 2070 Max-Q, 240Hz panel, dump on `D:`. The
  configuration that works best: Windows' per-application GPU preference
  pointed at the discrete card, `1440x1080`, `fullscreen = true`. That gives
  the dump's original `fp40`/`vp40` shaders, a 1:1 unscaled presentation and
  a flat 60.0 fps. On the integrated GPU the same resolution drops to the
  low 50s, which on this engine means the game runs slow.
- **Desktop** — Intel Iris Xe, dump on `C:`, also hosts the build toolchain
  under WSL. Has the Cg Toolkit installed system-wide, which is why it was
  the machine that caught `start.bat` converting shaders before the loader
  could.

## Remaining work, with research already done

### P4 — done: file and TTY redirects on Windows

`file_redirect` and `ignore_custom_ioctls` both work on both platforms now:
`platform/windows/files.rs` hooks LINE's `open`/`open64`/`fopen` stubs and
`platform/windows/ioctl.rs` its `ioctl` stub.

What the ioctl hook is worth, measured rather than assumed: LINE's
`stubs/ioctl.cpp` translates `TIOCMGET`, `FIONBIO`, `SIOCGIF*` and
`TIOCGSERIAL`/`TIOCSSERIAL`, and prints `Untranslated ioctl <n>` before
failing for anything else — so the game's two custom ops (`0x5463`/`0x5464`,
21603/21604 in that message) would fail where Linux reports success. But a
full-race `line_console.log` has not one such line, because the game reaches
those ioctls through `/dev/ttyM1` and `/dev/ttyM2` and `open` fails on both
first. So this is parity with Linux rather than a fix for anything observed,
and it is why the hook answers without calling LINE's stub at all: there is
nothing behind it worth calling. `tests/mockline` covers both halves.

MinHook does take this stub, which was the open question given `fopen64`:
a run on the desktop logs `ioctl: hooked ioctl()`.

One gap left: `fopen64` refuses to hook (MinHook declines the stub at
`0x40ef00`), so a little file activity is invisible. And LINE already rewrites
`/tmp/...` to `./tmp/...` itself in `fixPathIfNeeded`, so don't add that
rewrite again.

### P5 — custom BGM (built in, from `vixen256/audio_3dxp`, GPLv3)

~800 lines of Rust, hooks ~15 game symbols plus a few version-gated absolute
NOPs; reads `plugins/audio.toml` and `plugins/bgms/*`. Port into `src/bgm.rs`
behind a config flag, keep its file layout so existing packs work, and verify
expected bytes before writing the NOPs. Credit vixen256.

**Every symbol it needs is present in our Export dump**, checked one by one.
The two `clV363CardData` and two `clV337Card` names miss, but those are the
3DX and WM3 card classes and this build is V386, so the version branch that
wants them is never taken here. One name is not in `main` at all --
`_ZNSbIwSt11char_traitsIwESaIwEEC1EPKwRKS1_`, the `std::wstring(wchar_t
const*, allocator const&)` constructor -- and that is fine: it lives in
`libso/libstdc++.so.6` as a weak symbol and resolves at run time through
`dlsym` on Linux and LINE's `DlSym` on Windows, which is also how the game's
own call to it at `0x8541339` is bound.

**The dump carries the whole BGM catalogue in two tables, so nothing needs
hardcoding.** This is the thing worth knowing before porting: audio_3dxp
hardcodes three sets of songs with their paths and titles, and all of it can
be read out of the binary instead, correct for this build by construction.

- **`0x9321260` is `nsAudio`'s BGM table: 111 entries of 12 bytes**, indexed
  by `emBgmType` 0..110. `+0x00` `const char*` WAV path, `+0x04` `float`
  relative volume (2.5 for every music entry -- which is all `setRelativeVolume`
  forces, so that hook is just the table's own value), `+0x08`
  `const wchar_t*` title *already in the game's "N Name" display form*, e.g.
  `6 Inexhaustible Energy`. Entry 0 is `data/sound/bgm/maxi1/wm2_05.wav`,
  which is the path audio_3dxp keys its substitution on; 109 is
  `/tmp/sys_04.wav` and 110 `data/sound/bgm/test/test_attract.wav`, the
  `0x6E` that `get_bgm_index` special-cases.
- **`0x93e3560` is the menu's `int[3][25]`**, read as `table[set * 25 + index]`
  by `nsMenuCheck::getBgmSelectName`. Slot 0 of each row is 110 (the RANDOM
  entry), then the songs in menu order, then `-1` to end it. Set 0 is the
  3DX+/3DX/maxi3 soundtrack (20 songs), set 1 is maxi1+maxi2 (23), set 2 is
  the rival themes (12). audio_3dxp's hardcoded tables agree with this
  exactly, in content and in order, so they were read off a running game --
  but deriving them makes the port smaller and self-checking.

**The version-gated NOP is a clamp, and now has expected bytes.** For this
build (`W3P`/`EN2`/`B`/rev 2) it is `0x87EF4FC` in
`clSeqCardDisp9::run`, and the three bytes are `0f 4d c2`, `cmovge eax, edx`
in:

    movzx  eax, BYTE PTR [ecx]     ; the BGM set
    mov    edx, 2
    cmp    al, 3
    cmovge eax, edx                ; set >= 3 ? 2 : set
    mov    BYTE PTR [eax+0x16d], dl ; ... into the card

So the game clamps the stored set to at most 2 on its way into the card at
`+0x16D` -- the same offset audio_3dxp reads and writes -- and NOPing the
`cmovge` is what lets a custom set index survive. Verify `0f 4d c2` before
writing.

**Why the plugin has a naked assembly trampoline, and how to avoid it.**
`clAudioStream::play(emBgmType)` loads `ebx` with `table[id].path`, calls
`clWaveFile::loadFile(&local, ebx)` to read the header, and then **`fopen`s
`ebx` again** at `0x8540ff5` to stream the body. `ebx` is callee-saved, so a
hook on `loadFile` that substitutes a path gets the header from the new file
and the stream from the old one; the trampoline exists only to force `ebx`
after the fact. Swapping `0x9321260`'s entry 0 for the duration of the
original `play` call instead feeds both reads from the table and needs no
assembly -- and swapping the title pointer with it means `getMusicTitle`
needs no hook either.

**Assembly is still needed for one thing: the string returns.**
`nsAudio::getMusicTitle` and `nsMenuCheck::getBgmSelectName` return
`std::wstring` by value, which on the i386 Itanium ABI means a hidden sret
pointer as the first argument *that the callee pops* -- both end in
`ret 0x4`, confirmed at `0x8541345`. audio_3dxp gets this right by accident
of writing `-> NsAudio`, since Rust's own i686 System V ABI returns every
struct through sret and emits `ret 4`. **That is a Linux-ABI coincidence and
must not be relied on for `i686-pc-windows-gnu`**, where the Microsoft cdecl
rule is that the caller pops the hidden pointer. Hook those two through an
explicit trampoline that takes the sret pointer as a plain argument and ends
in `ret 4`, the way `shim/call.c` already handles the other ABI mismatches.

**Dependencies the port does not need.** `rand` (a small PRNG is enough and
is unit-testable), `widestring` (the titles are NUL-terminated `u32` arrays,
which is a `Vec<u32>`) and `glob` on Windows, where it is a Linux-only
dependency today and `std::fs::read_dir` does the job. `hook.rs` drops out
entirely: it is `hook::hook_symbol` and `platform::write_memory` already.

**And every call into the game must go through `call_game`.** audio_3dxp
calls its trampolines directly, which is correct on Linux and faults in SSE
code on Windows. That is the single most mechanical part of the port and the
easiest to half-finish.

**The card screen's BGM row, and how a custom set reaches it.** Card check
page 8 is `clSeqCardDisp9` (`run` at `0x087ef0ec`), a three-row settings
editor whose rows are an `int[3]` at `[this+0x44]` with their maxima at
`[this+0x5c]`: row 0 is the BGM volume (`card+0x16C`, an
`emBgmVolumeType`, four levels, default 2, consumed by `mseqEntry` at
`0x088d375a` through `SNDStreamSetVolumeTypeFromCard`), **row 1 is the BGM
set** (`card+0x16D`), row 2 is the print-pass type. The cursor row is
`[this+0x40]`.

Input is `clMenuInput::chkSW(mask)` at `0x0891a13c`, which is worth knowing
on its own: **it is the single funnel every menu in the game reads buttons
through, 295 call sites**, and it synthesises its bits from two fields of the
singleton at `0x097fe8b0` -- `[+0x10]` giving bit 1 (left) at `-1` and bit 2
(right) at `1`, `[+0x14]` giving `1`->`0x10` gas, `2`->`0x20` brake,
`3`->`0x40` view, `4`->`0x80` intrude. `exec()` at `0x0891a20c` fills
`[+0x10]` with `trunc([dev+0x20] * 3.0)` clamped to +-3, where `[dev+0x20]`
is the steering float. So **menu cursors are the wheel, left and right only;
this game's menus have no up or down**, which is why the row is scrolled
sideways and why nothing needs a JAMMA bit injected.

Three things kept custom sets out of that row, all handled in `src/bgm.rs`
under `bgm_selector`:

- **The maximum is an unlock count.** `[this+0x68]` is 0, then 1 if
  `card+0x279` is set, then 2 if `card+0x186` is set, reaching `[this+0x60]`
  through a three-way dispatch at `0x087ef1bf`. Forcing the two lower writes
  (`0x087ef16c`, `0x087ef192`) to 2 makes every path take the `== 2` branch,
  which is also the branch that loads the full
  `menu_g_cardcheck_8_sound_sel` plate instead of the cut-down (`_2`) or
  locked (`_no`) ones -- so one change unlocks the row *and* gives it artwork
  with three blocks of frames. Only that branch's store at `0x087ef936` then
  needs the real count. **What `card+0x186` and `card+0x279` actually are is
  still unknown**; forcing past them means the row is offered on cards where
  the game meant it locked.
- **Row 1 clamped.** Only row 2 wrapped, and the test is the row index
  against 2 in both directions (`0x087efd1a` up, `0x087efedf` down). `cmp
  reg,2; jne` becomes `cmp reg,1; jl`, so rows 1 and 2 wrap and row 0 still
  clamps. Move the immediate without the condition and row 0 starts wrapping.
- **The artwork is frame blocks, not images.** Each set plays 59 frames of
  one `.anm` at start 0, 100 or 200, and for an index of 3 or more the
  dispatch makes **no `setAnime` call at all**, leaving the previous set's
  frames on screen. So a custom set borrows the third block from an
  `an_reset`/`initAnime` hook, and the sprite *descriptor* that block draws
  from is repointed at the pack's `mod.png`. **The card screen never goes
  through `Gap::Gfx::igImage::load`**, so the hook that substitutes the
  pre-race logo cannot reach it: `menu_p_sound_logo_maxi*.png` has zero code
  references, and sprites here resolve through `clImageResourceManager::load`
  (`0x08519a88`) from ids compiled into the `.anm`. The descriptor table is at
  `0x0955aaa0`, stride `0x2c`, `0x1fad` entries, handed over in
  `clMain::clMain()` at `0x088c7d00`; an entry is `{ const char *path, int,
  int, int, float w, h, sx, sy, w2, h2, int }` and `maxi1/2/3` are ids
  `0x1650/0x1651/0x1652` at `0x09598060/0x0959808c/0x095980b8`, all 256x32.

**Still unproven:** that frames 0-58/100-158/200-258 of that plate are
maxi1/maxi2/maxi3. The `.anm` encoding was not decoded -- the raw dwords
`0x1650/0x1651/0x1652` do not appear in the file, so ids are stored narrower
or table-encoded. If the assumption is wrong, a custom set shows the wrong
plate and repointing the descriptor has no visible effect.

### P6 — Gemballa unlock (from tattoohanz's Gemballa-Unlocker)

Extracted from the PyInstaller bundle the user supplied. Two halves:

**(a) `main` byte edits** — apply in memory after verifying the expected bytes.
File offset + `0x8048000` = virtual address. **3DX+ Export v386**, 24925524
bytes. **The md5 recorded here was `6f02c8bc3c6c1711eb2bb97218328ff6` and no
dump we have matches it** — two independent Export copies both hash
`aac96862372ee774a14096792fc40c13` at that exact byte count. Every one of the
seven offsets below verifies against them, so the offsets are right and the
hash is not; it was presumably taken from a patched copy. Treat the md5 as
unreliable and the expected bytes as the real identifier, which is what
`src/gemballa.rs` does:
- `0x7A8641`: `83 be b0 02 00 00 01` → `...00` — maker-select lower bound,
  lets index 0 (GEMBALLA) be selected. *This is the region lock.*
- `0x7A8209`, `0x7A86AB`, `0x7A87A8`, `0x7A88B3`:
  `c7 44 24 08 01 00 00 00` → `c7 44 24 08 47 1c 00 00` — the four
  `setImageReplace(gemballa, NULL)` calls become identity.
- `0x6895A8`: `83 c8 02` → `90 90 90` — `clV386CardData::assign()` stops
  OR-ing error flag 2 for Gemballa cards, which otherwise makes the card
  invalid.
- `0x12ED6B0`: dword `277` → `247` — story chara 30 drives the Gemballa
  again. **It is Shima, not Masaki, and the car is his Blackbird**: resolving
  the car-id table absolutely — 311 records of 24 bytes with the name pointer
  at `+8`, laid out identically in both builds — gives 247 as
  `gem_964_38RS_Shima1st`, 251 as `gem_964_RSR_Masaki`, and Export's 277 as
  `nis_BNR32_Feeling`. The "Masaki" label here was wrong and was believed
  twice before an in-game observation forced the check; `nis_Z33_Shima` is a
  *different* entry, chara 20, identical in both builds, which is what makes
  the mistake easy to keep making. **He is the only one.**

  **And it does not affect story mode.** Tested on hardware on 2026-10-17 by
  setting *all 32* entries of that table to 247 and playing story chapter 6/7:
  Shima still arrived in the Z33, with the HUD captioning him "Tatsuya Shima
  / Black Bird" over a Nissan. So the game does not read this table for story
  rivals, and whatever `0x12ED6B0` governs, it is not the chapter that the
  Blackbird appears in. The 32 entries pair with a 32-pointer array of "vs
  <name>" labels at `0x138BFC0` whose index 30 is "vs Tatsuya Shima", so the
  table is a rival roster for *something* -- just not story mode.

  **Where story mode gets its cars is unknown.** `clSeqStEnemyInformationThread`,
  `clSeqStEnemyLevelSelectThread`, `seqRaceStory` and `mseqStory` are the
  machinery.

  **Found: the story rival's car is 35 dwords in `.data`.** The Export build
  holds model id 277 (`nis_Z33_Shima`) where a JPN dump holds 248
  (`gem_964_38RS_Shima2nd`), at the same offsets within the section, and
  restoring the JPN value puts the Blackbird back -- confirmed in Episode 7
  on hardware. `src/gemballa.rs` does this under `gemballa`, and the offsets
  are listed there.

  Both ids come from `car_log` at run time, not from the car table. **Every
  static walk of that table was off by one**, in one direction or the other,
  and twice produced a confident wrong answer -- including "Masaki" for an
  entry that is Shima's. Treat the table as something to explain a runtime
  observation with, never as the source of one.

  What ruled the rest out, each by experiment: setting all 32 roster entries
  to one car (nothing changed), setting every `.rodata` dword holding 277 to
  another value (nothing changed, and it broke the id-to-name map at
  `0x013DCD24`, which is how that map was identified), and hooking all three
  `nsCarParts` id-space converters (never called, so the id is stored
  directly). The chain is `LuaNickNameGetFromModelID` <- `constructBody` <-
  `loadBody` <- `callBodyConstructParam`, with the id at
  `clCarConstructParam+0x08`; the player's car takes the identical path, so
  none of those frames tells a rival apart. Diffed against a JPN dump on
  2026-10-17 (`H:\wmmt3dxp`, `main` 24,855,892 bytes): of the 32 story
  characters, exactly one car id differs between the builds. Nobody asking
  "does it change Shima's car too?" needs to wonder again — no, and no one
  else's either, so tattoohanz's single edit is complete rather than partial.
  Fields 2–4 of every record *do* differ, consistently 11–17 lower in JPN,
  which is why they are not car data but string or dialogue indices that
  shifted when the Export build added content.

  **The table can be found by shape rather than by offset**, which is worth
  preferring over the hardcoded address above. It is 32 records of 24 bytes,
  `[car_id, 39, a, b, c, 0]`, and scanning for runs where field 1 is `39` and
  field 5 is `0` finds exactly one such run in each binary — `0x012ED3E0` on
  3DX+ Export, `0x012EB660` on the JPN dump — with the Export hit landing
  precisely where Masaki's documented offset says the table starts. A search
  costs a few million dword compares once at startup and makes this edit
  build-independent, where an offset is one revision away from being wrong.
- Optional card-conversion NOPs at `0x6825F4`, `0x683CA7`, `0x684FD5`,
  `0x68666E` (only matter for v337/v363 cards upgrading to v386).
- 24 frame-table immediates at `0x7A8028`–`0x7A8130` (`c7 45 xx` stack
  stores). **Default: leave alone.** They only need changing if you also
  replace the `.anm` timelines with JPN-derived ones.
- A separate 3DX profile exists (`DX_SIZE` 24288148, maker index at
  `[esi+0x494]`, `clV363CardData`, one byte edit at `0x1390E90`: `35`→`34`).

**(b) Asset porting** — **three car folders only, 166 files.** Measured
against a JPN dump on 2026-10-17, and narrower *and* harder than it looked:

| | 3DX+ Export | JPN |
|---|---|---|
| `data/car/gem_964_38RS` | directory exists, **0 files** | 57 |
| `data/car/gem_964_RSR` | **0 files** | 67 |
| `data/car/gem_997_AVA` | **0 files** | 42 |
| `data/characar/gem_*` (Shima1st, Shima2nd, Masaki, Kijima, Normal) | complete | byte-identical |
| 92 gem-named menu sprites and `.anm` | present, under `sprite-us`/`anime-us` | 50 identical, 25 localised |

So Namco shipped Export with the Gemballa **menu art and rival cars intact**
and stripped only the **player-selectable models**, then locked the maker.
That is self-consistent: the story needs Shima's 964 whether or not you can
buy one.

Two consequences. The car folders genuinely cannot be replaced by code and
need a JPN dump, so keep that an offline step. But **do not copy the sprites
or `.anm`** — the Export dump already has all 92, and 25 of them differ only
because they carry English text. Copying the JPN ones would replace the
localised menu art with Japanese for no gain, which is what
`copy_gemballa_assets` would do if pointed at the whole tree.

**Optional dialogue fix** (from tattoohanz's *widescreen* installer manifest,
same 3DX+ Export target): write 3 UTF-32 strings into free space at
`0x1388971`, `0x1388999`, `0x139CD48` and repoint 3 pointers at `0x58EA0C`,
`0x58EA66`, `0x5900E6`.

**What his C++ loader's `probe.cpp` adds**, which is the symbol-level view of
the same thing and is how to confirm the byte edits did what they were meant
to rather than trusting an offset. Its goal was to settle empirically which
screen drives the maker list and what index each cursor position asks for:

- `get_car_from_maker(maker, car)` is the function the lower bound guards,
  returning a table of `{count, ids...}`.
- Maker order is `GEMBALLA = 0`, then MAZDA, MITSUBISHI, NISSAN, SUBARU,
  TOYOTA — so the lower bound of 1 is exactly what hides index 0.
- Two candidate drivers, `clSeqCardMakerSelectThread` and
  `clSeqTgWheelMakerThread`, with the cursor at `[this+0x494]` and the previous
  value at `+0x490` (verified on clean 3DX only).
- Everything resolves by mangled name, so it works unchanged on
  3DX / 3DX_JPN / 3DX_exp: `_Z18get_car_from_makerii`,
  `_ZN26clSeqCardMakerSelectThread3runEPv`, `_Z18seqCardMakerSelectv`,
  `_ZN24clseqCardCarSelectThread12change_modelEN10nsCarParts12emCarModelIDES1_`,
  `_ZN23clSeqTgWheelMakerThread3runEPv` and the rest of that family.

Take the facts, not the code: it classifies return addresses against
hand-written per-build function size tables keyed on a hardcoded
`get_car_from_maker` address, and `src/elfsym.rs` now gives sizes from
`st_size` for every build for free.

### P7 — network

Currently the network boot thread is stubbed out unless `local_ip` is set.
`clNet::getAddress` already returns `local_ip`, and since 2026-10-06 it reads
the net struct from the offset the revision calls for (`+0x1C` on base WMMT3,
`+0x24` on 3DX and 3DX+) rather than guessing, and reports rather than
panicking when it cannot answer. **The /24 mask at `net+0x08` is still not
written** — his loader writes `255.255.255.0` there, and that is the next
thing to do. Then try two instances on loopback for split-screen.
`clNet::setInterfaceAddress` is already stubbed on Windows. If the game's LAN
session needs broadcast, loopback won't carry it — say so and move to real IPs.

His loader also carries a LAN-VS diagnostic block worth knowing exists, though
it is installed only on base WMMT3 and we do not target that: socket lifecycle
tracing on LINE's `socket`/`bind`/`listen`/`accept`/`connect`/`getsockopt`
stubs, a chokepoint on `boost::asio::detail::throw_error` (which carries the
error value), and hooks on `clNet::startJoinPlay`,
`clLanSessionPimpl::connectHandler`, `clNet::sendCrownUnlock`, `seqMain` and
`mseqPlay`. The intent was to run the same trace on a working DX cabinet and
diff, which is the shape this phase should take if the handshake turns out to
be the problem.

## Fixed: the card printer could end a race

All of this is tattoohanz's measurement, from his `card.cpp` and `card_diag.cpp`
comments; what we had was upstream's six-command version, which had every one
of these bugs. It matters out of proportion to its size because the card path
is the one a race ends on, and nobody had run one.

Five of the thirteen RPS commands hit `panic!`, and `panic = 'abort'` makes
that the process. `PRINTSTRING 0x7C` is the text the reader would burn onto
the card -- the mileage banner at the end of a race. `EXTRACHAR 0x7A`,
`ERASE 0x7D`, `CLEANING 0xA0` and `BATTERYCHECK 0xF5` are the rest of the same
print state machine, which `clCardDeviceGameService::clPrintProcess::run`
drives by polling `chkFinish()` and `chkNoError()`. `BATTERYCHECK` must answer
`0x30` in s2: `0x40` is how a real reader reports a flat backup battery.

`clCardPrinter::startSequence()` **zeroes s1/s2/s3 before every command**, and
`0x00` is not `0x30`, so a command that answers without writing a status
leaves `chkNoError()` false. `READSTATUS` -- which the boot sequence relies on
-- wrote none at all, handing the game whatever `initSerial()` left behind,
including a negative connect error at `[this+0x1c]` if opening the serial
device failed. **That is the intermittent "card reader not functional" on the
title screen.** `clCardDeviceGameService::clStatus::setupResult()` only reaches
`0xC`, the one value `clSeqBootCardDeviceThread` accepts, when every `chk*` is
false and `chkNoCard` is true.

The printer object, as `clCardPrinter::getRPStoStatus` reads it: pending
command word at `+0x00` (write 0 to finish), request buffer `+0x08`, response
buffer `+0x10`, s1/s2/s3 at `+0x04`/`+0x05`/`+0x06`, signed connect error at
`+0x1c`. `getReadData()` is `memcpy(dst, this[0x10] + 6, 0xcf)`, so the card
bytes go six into the response buffer.

`card.bin` is `0x45 * 3` = 207 bytes, three tracks of 69, with a
CRC-16/CCITT-FALSE at `0xCD` over bytes `0..0xCD` -- from `clV386CardData`'s
`getSumStart` → `0xcd`, `getSumEncStart` → `0`, `getSumEncSize` → `0xcd` and
an assertion of `0xcf`. **The game writes tracks individually**:
`requestWrite()` diffs against the old card and sends selector `0x30` (track 1,
69 bytes at 0), `0x35` (tracks 2-3, 138 at `0x45`) or `0x36` (all 207 at 0).
Since the sum lives in track 3, a partial write over a zero-filled buffer
produces a correctly sized card with dead tracks and a sum matching nothing --
and one `EJECT` that cleared the buffer followed by any partial write bricked
the saved card permanently. `EJECT` now reloads `card.bin`, because a real
reader spits the card out and the player pushes it back in.

**Open question, deliberately:** we report a checksum mismatch and save
anyway. The sum covers all three tracks, so a track-1 write ought to change it
-- yet the game sends track 1 alone, so either the wire format is more than
this or the sum is kept some other way. Refusing to save on a mismatch (which
his code does) would silently throw away a race if that assumption is wrong,
and the bytes are the game's own either way.

3DX and 3DX+ added `clCardDeviceGameAutoService`, absent on base WMMT3, which
polls the device every frame. Its `getResult` starts at pending and latches a
hard error once enough polls pass with nothing answering, which calls
`clNet::stop()` and draws `clMsgCardDeviseError` -- **what an idle attract loop
eventually reaches**. It reads `[svc+0x34]` (busy) and `*[svc+0x2c]` (the
status, whose first int is the result) itself and never calls
`clCardDeviceGameService::getResult`, which is why hooking that getter sees
nothing when the error arrives this way. The result codes, from the messages
each one draws:

| | | | |
|---|---|---|---|
| 0 idle | 1 busy | 2 dispenser empty | 3 connect — check connections |
| 4 write error — save failure | 5 read error — partial read | 6 block error — card jam | 7 print error |
| 8 system error | 9 illegal error — card jam | 0xA cannot execute — ejection error | 0xB battery too low |
| 0xC no card, no error — **OK** | | | |

## Fixed: the game ending itself

For as long as anyone has run this, the game would sometimes just close --
no error, no crash dump, nothing in any log. It is an unhandled **SIGALRM**.

The game arms `setitimer(ITIMER_REAL)` for every 20ms -- 50 Hz -- and installs
a handler for SIGPIPE and nothing else. There is no `signal(14)` and no
`sigaction(14)` anywhere, measured with both hooked from before `main` ran and
with 44 of the 64 deduplication slots still free, so that is a real negative
and not a dropped line. LINE stubs none of `signal`, `sigaction` or
`setitimer`, so all three are Cygwin's own. Whatever absorbed that signal on
the arcade's Linux, here it eventually reaches a thread that does not block it
and the default action ends the process.

`ignore_alarm` (default on, `platform/windows/alarm.rs`) sets `SIG_IGN` for
SIGALRM during init, before `shutdown.rs` hooks `signal` so the log does not
read as though the game had done it. Nothing in the game can be listening, so
nothing is lost; a version that installed its own handler would simply replace
ours.

**What it bought, measured:** before, an idle attract loop ended the process
every time -- at 42s, 75s and 82s across three runs. After, it rendered for 29
minutes at a flat 60fps and was still going when the run was stopped from
outside; no ending signal of any kind was logged.

**How it was found, which is the part worth keeping.** Three things had to be
built before the question could even be asked, and all three are in the tree:

- `shutdown.rs` hooks all four of msys's exit functions. The *negative* result
  is what mattered -- none of them ever fires -- because it ruled out `exit()`
  and left only a signal.
- The process exit code, which every launch had been discarding. Cygwin encodes
  a normal exit as `status << 8`, a signal death as `sig << 8`, and adds `0x80`
  when it dumps core. `0x8B00` decodes as SIGSEGV-with-core and matched a run
  that had a stackdump, which is what made `0x0E00` readable as signal 14
  rather than `exit(14)` -- the two are identical from the code alone.
- `shutdown.rs` also watches `setitimer`, `signal` and `sigaction`, which is
  what showed the 50 Hz timer with nothing listening.

Worth knowing for the next one of these: an access violation goes to the
vectored handler in `crash.rs` and a Cygwin fatal signal writes
`line.exe.stackdump`, so an ending with *neither* was never a crash. The
Windows event log is also silent for both, since nothing faults.

## Fixed: `dist/start.sh`'s destructive recompile

The Linux launcher used to write its `.recompiled` marker *before* doing any
work and delete each `.fp`/`.vp` with no backup, so one run without `cgc` left
the game with no shader files at all and then permanently skipped the retry.
That block is gone: `shader.rs` does the job with the compiler inside
`libCg.so`, which now ships beside the launcher, so `cgc` need not be
installed. `shader.rs` also recovers the folders the old block wrecked — it
rebuilds when the compiled programs are *missing*, not only when they carry
`OPTION NV_`, which is the case it could not previously see.
