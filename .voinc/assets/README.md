# Brand assets

Canonical copies of the fork's logo and of every icon asset derived from it.
`.voinc/rebrand.sh` (section 3b) copies these over the tree on every sync,
always replacing upstream's artwork at the same paths; the same bytes are
committed on the `voinc` branch, so an in-place re-run changes nothing.

The mark is `assets/logo/logo-transparent.svg` (white on alpha); `logo.svg` is
the same path on a black plate. One path, `fill-rule="evenodd"`, all commands
absolute: its first `M`-subpath is the nib (with the tall centre spikes), the
remaining six are the outer floating bars.

Derived files and their rules (plate masks and colors are taken from
upstream's own icons at the same paths):

- `icons/hicolor/<s>x<s>/apps/io.voinc.voinc.png` — Linux: rounded dark plate
  (alpha mask of upstream's 512 icon, flat rgb(46,46,46)), the mark sized to
  the old glyph's area, centred. Full mark at 512/256/128/64; the nib alone at
  48/32/24/16, at the old glyph's 77% of the plate height.
- `windows/voinc.ico` — same plate at 256 (mask of upstream's 256 layer, flat
  rgb(22,22,22)); full mark at 256/64, nib at 48/40/32/24/20/16 at 77% of the
  plate. Layer layout matches upstream's file: PNG blob for 256, 32bpp BMP
  with a 1bpp AND mask for the rest (~52 KB, upstream's was ~51 KB).
- `windows/icon-{32,64,128}.png` — white mark on transparent, fitted into the
  old glyph's box (nib at 32).
- `windows/tray-32.rgba` — 32×32 raw RGBA, white mark, alpha = shape; the nib
  fitted into the old mark's box (1,0)–(31,32). `src/desktop/tray_icon.rs`
  recolors it black on light panels.
- `macos/AppIcon-Image.png` — the full mark fitted into the old 1488×1672
  Icon Composer layer canvas, white on alpha; `icon.json` is untouched.
- `voinc-icon.png` — 192×192 plate as hicolor (mask of upstream's file, flat
  rgb(57,57,57)); full mark at the old glyph's area.

Regenerating (only when the mark itself changes): render the SVG at 3628 px
with inkscape, trim to the alpha bbox, force straight white RGB, and
recompose by the rules above (Lanczos); split subpaths on `M`. The tray and
the 32 px glyph are written raw (4096 bytes). Tooling used: inkscape 1.x and
Pillow; no other image tooling is needed.
