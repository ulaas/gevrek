# gevrek brand assets — usage

Logo, app icon and shell icon set for **gevrek**, a cross-platform window manager and desktop shell.

The mark is a gevrek seen from above: the ring's twist is cut into five slanted panes (tiled windows), and one pane is lit in sesame (the focused window).

## What's in the pack

```
logo/
  mark/         gevrek-mark-{on-dark,on-light,on-crust,mono}.svg         five-pane mark
                gevrek-mark-small-{...}.svg                              three-pane mark, 32 px and under
  horizontal/   gevrek-logo-horizontal-{on-dark,on-light,on-crust,mono}.svg  (+ .png, 512 px tall)
  stacked/      gevrek-logo-stacked-{on-dark,on-light,on-crust,mono}.svg     (+ .png, 800 px tall)
  wordmark/     gevrek-wordmark-{on-dark,on-light,mono}.svg
app-icon/
  macos/        gevrek.icns, gevrek.svg, gevrek-small.svg, png/gevrek-{16…1024}.png
  windows/      gevrek.ico,  gevrek.svg, gevrek-small.svg, png/gevrek-{16…256}.png
  linux/        hicolor/<size>x<size>/apps/gevrek.png, hicolor/scalable/apps/gevrek.svg
  (each also has a -nightly variant on the crust-orange ground)
favicon/        favicon.ico, favicon.svg, favicon-{16,32,48}.png, apple-touch-icon.png
icons/
  symbolic/     gevrek-<name>-symbolic.svg   24 icons, currentColor
  color/        gevrek-<name>.svg            same icons, sesame stroke + ember accent (for dark UI)
  gevrek-icons.sprite.svg                    all 24 as <symbol>s
  icons.json                                 name, group, file, hasAccent
tokens/         gevrek.css (custom properties), gevrek.json
```

All PNGs have transparent backgrounds. The wordmark is outlined to paths, so no font is needed to render any logo file.

## Which logo file to use

| Background | File suffix | Panes | Focus pane | Wordmark |
|---|---|---|---|---|
| Dark (molasses `#21110A` or similar) | `on-dark` | crust `#D9701F` | sesame `#F4E2BC` | sesame |
| Light (sesame, white) | `on-light` | deep crust `#C75F14` | molasses | molasses |
| Crust orange | `on-crust` | molasses | sesame | molasses |
| Anything else, single ink | `mono` | `currentColor` | `currentColor` | `currentColor` |

- `mono` files inherit colour from CSS `color` when inlined; as `<img>` they render black.
- Prefer the **horizontal** lockup. Use **stacked** for square spaces and the **mark** alone where the name is already present (title bar, about dialog header, avatar).
- **Size rule:** use the five-pane mark at 48 px and above, the three-pane `small` mark at 32 px and below. The gutters of the five-pane mark close up at small sizes.
- **Clear space:** keep at least one quarter of the mark's diameter free on every side. The lockup SVGs already include this as padding.
- **Minimum size:** horizontal lockup 24 px tall; mark 16 px (small variant).

Don't recolour individual panes, rotate the ring, move the lit pane, add effects (shadow, gradient, outline), or re-typeset the wordmark in another face.

## App icon

Each platform has its own container; the mark and colours are the same.

| Platform | File | Shape |
|---|---|---|
| macOS | `app-icon/macos/gevrek.icns` | rounded square, 824 px body on a 1024 canvas (Apple icon grid) |
| Windows | `app-icon/windows/gevrek.ico` | full-bleed soft tile; contains 16, 24, 32, 48, 64, 128, 256 |
| Linux | `app-icon/linux/hicolor/` | circle; PNG 16–512 plus scalable SVG |

Sizes 32 px and under inside the `.ico`, `.icns` and hicolor PNGs already use the three-pane mark.

`-nightly` files (crust ground, molasses panes) are for dev and nightly builds so they can be told apart from a release in a dock or taskbar.

**Linux install**

```sh
cp -r app-icon/linux/hicolor/* ~/.local/share/icons/hicolor/     # or /usr/share/icons/hicolor/
gtk-update-icon-cache ~/.local/share/icons/hicolor/ 2>/dev/null || true
```

Then reference it by name in the `.desktop` file: `Icon=gevrek`.

**Windows** (resource script)

```rc
IDI_ICON1 ICON "app-icon/windows/gevrek.ico"
```

**macOS** — copy `gevrek.icns` to `Gevrek.app/Contents/Resources/` and set in `Info.plist`:

```xml
<key>CFBundleIconFile</key>
<string>gevrek</string>
```

**Web / docs site**

```html
<link rel="icon" href="/favicon.ico" sizes="48x48">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
```

## Shell icons

24 icons on a 24 px grid, 1.75 px stroke, round caps and joins, no fills except the accent part.

| Group | Icons |
|---|---|
| window | `close` `minimize` `maximize` `restore` `fullscreen` `float` |
| layout | `tile-left` `tile-right` `master-stack` `grid` `stacked` `workspaces` |
| panel | `launcher` `search` `terminal` `files` `settings` `notifications` |
| system | `network` `volume` `battery` `clock` `lock` `power` |

**Symbolic set** (`icons/symbolic/`) uses `currentColor` for everything, so it follows the text colour and works with GTK/Qt symbolic recolouring. File names follow the freedesktop `-symbolic` convention.

Six icons have an **accent part** — the filled shape that marks the active piece (the chosen half in `tile-left`, the current workspace, the battery level, and so on). It carries `class="gevrek-accent"`. When the SVG is inlined or used from the sprite you can tint it:

```css
.gevrek-icon { color: var(--gevrek-sesame); width: 24px; height: 24px; }
.gevrek-icon .gevrek-accent { fill: var(--gevrek-ember); }
```

```html
<svg class="gevrek-icon" aria-hidden="true"><use href="gevrek-icons.sprite.svg#gevrek-tile-left"/></svg>
```

Note: styling `.gevrek-accent` reaches into `<use>` content only when the sprite is inlined in the same document. For external files or `<img>`, use the pre-coloured `icons/color/` set (sesame stroke, ember accent, meant for dark surfaces).

**Sizing:** draw at 24 px, or 16/20/32/48 px. At 16 px keep the stroke from going under 1 px by rendering the 24 px artwork at 16 px rather than redrawing it thinner.

**Accessibility:** icon-only buttons need a text label (`aria-label`, tooltip, or the toolkit's accessible name). Decorative icons should be hidden from assistive tech (`aria-hidden="true"`).

**Adding icons:** stay on the 24 px grid with a 2 px margin, 1.75 stroke, round caps and joins, corner radii of 1.5–2.5, and at most one filled accent shape per icon.

## Colour

| Token | Hex | Use |
|---|---|---|
| `--gevrek-molasses` | `#21110A` | dark ground, text on light |
| `--gevrek-crust` | `#D9701F` | the ring, brand colour |
| `--gevrek-crust-deep` | `#C75F14` | the ring on light grounds |
| `--gevrek-ember` | `#E58A3C` | accent and small text on dark |
| `--gevrek-sesame` | `#F4E2BC` | focus, light ground, text on dark |
| `--gevrek-surface` | `#2E1A10` | raised tile on molasses |
| `--gevrek-muted` | `#C4A482` | secondary text on dark |
| `--gevrek-muted-on-light` | `#5E3D26` | secondary text on sesame |

Crust `#D9701F` is for shapes and large type. For small text on dark use ember; on sesame use molasses or `muted-on-light`.

## Type

- **Outfit SemiBold (600)**, lowercase, −3.5% tracking — the wordmark and headings. Open Font License; the wordmark files are already outlined.
- **JetBrains Mono** — labels, docs, CLI text. Open Font License.

The name is always written lowercase: **gevrek**.
