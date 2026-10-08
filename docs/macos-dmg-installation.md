# Pipkin's drag-to-Applications disk image

## Direction contract

**Mode / thesis:** Operate. One familiar Mac drag installs Pipkin; the approved Pipkin mascot
personally explains it. This extends the existing brand, not the app's visual identity.

**Own-world:** Existing Poppins, approved unaltered focused mascot, quiet cool-neutral ground,
white speech balloon and restrained `#3454d1` blue. No invented mascot pose, recolouring,
fake install button, percentage or painted substitute for the real Finder items.

**Story:** “Drag Pipkin into Applications.” Then open it from Applications and eject the disk
image once the copy finishes. Neither the artwork nor packaging claims notarization or
publisher trust.

**First viewport:** An 800×520-point Finder icon-view window. Mascot upper left, instruction
balloon upper right. Real 128-point Pipkin.app and Applications shortcut icons sit at
(260,322) and (580,322), joined by one blue arrow. Secondary launch guidance sits inside
the balloon; ejection guidance is quiet at the bottom. Main copy never occupies icon or
label regions.

**Form:** A tightly specified extension of the owner's accepted DMG, not an open concept
round. Code-led composition of existing approved artwork and fonts; no image generation.
Native Finder owns drag/drop, app/folder icons, labels and window controls.

**Finish:** unreviewed and undocumented is unfinished; this build ends with the finish
review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.
The existing `docs/DESIGN.md` world is inherited unchanged. Native Finder aesthetic/drag
acceptance is a separate owner gate, not implied by passing metadata tests.

## Assets and regeneration

- `packaging/dmg-background.tiff`: lossless 800×520 / 1600×1040 raster representations,
  respectively 72/144 dpi. Both decode to the same 800×520 logical-point size.
- `packaging/dmg-layout.json`: matching window, background path and real icon coordinates.
- `scripts/build-dmg-background.py`: rebuilds from the approved `assets/brand/mascot.png`
  and bundled Poppins fonts using ImageMagick. Needed only for asset regeneration.
- `scripts/test-dmg-background.py`: standard-library TIFF density/size/layout-clearance
  checks; on Mac it additionally checks the actual AppKit decoder without opening a window.

The mascot is proportionally scaled and alpha-composited, not redrawn or recoloured. The
source is 420×436px; its 168×174-point placement has enough source pixels for the 2x render.
The background intentionally contains **no app or folder icon pictures**: Finder renders
actual draggable items over it.

## Packaging behavior

`package-macos.py` stages hidden `.background/install.tiff` and writes root `.DS_Store`
on the writable HFS+ image before UDZO conversion. The settings select icon view, designed
bounds/positions, no auto-arrangement, and hidden sidebar/toolbar/status/path/tab bars.
The mounted-volume Pipkin icon remains `.VolumeIcon.icns` with its custom-icon flag.

A background alias records HFS+ volume/file identities and a volume-relative target;
it omits the build mountpoint/backing-image alias. The converted image is mounted read-only
and its actual background checksum, Finder settings, icon coordinates and alias identities
are verified. The existing copied-app signatures and private bare-PATH engine probes remain
in place. No artwork is added to or changed inside the already signed app bundle.

Pure-Python `ds_store` 1.3.3 and `mac_alias` 2.2.3 build tools use exact wheel URLs/hashes in
`packaging/dmg-layout-tools.json`. Their MIT notices/API code were inspected in those pinned
wheels. They are fetched only for DMG creation, extracted into disposable private directories,
and executed with Python `-I -S -B`. No global pip, npm, Node or Python environment changes.
They are build inputs, not modules bundled in Pipkin.app.

No Finder AppleScript, UI scripting, Automation consent or owner Finder preferences are
required. Older `bless --openfolder` is used only when the local tool advertises it, against
the owned image only, never with privilege elevation or setBoot. Otherwise a warning states
that auto-open is unverified. Finder opening/mounting behavior on current macOS remains a
native interaction gate. `--app-only` source installs skip all DMG assets/tools and downloads.

## Verification

**Local automated:** background's two density/size representations and icon clearances;
binary DS_Store settings and alias relocation fixtures; exact hash-pinned tool extraction,
private cleanup and unsafe-member/checksum rejection; package fixtures for missing/tampered
background/settings, original app/resource seals, Applications shortcut, volume icon, failed
attach cleanup, offline probe/socket budget and app-only bypass.

**Artwork evidence:** the generated raster's OCR reads all three instructional lines without
clipping. Fonts and contrast use the existing brand's Poppins and restrained blue/dark ink on
a light ground. The available harness cannot open PNGs for visual review. This is not a
Finder screenshot or owner acceptance of the illustration. A fresh read-only finish review
returned **ship at source/artifact scope**, with no material fixes required: TIFF densities,
geometry, icon/label clearance, wording, provenance and preserved packaging contracts checked.
Measured contrast: main instruction 15.73:1, blue emphasis 6.30:1, secondary guidance 7.72:1,
and arrow 5.83:1. It explicitly withheld native visual/drag/auto-open approval.

**Native CI added:** actual AppKit TIFF decoding and layout/package fixture logs precede
the existing macOS source install, signed ZIP/DMG verification, mounted-image layout check,
and copied-app probe. A CI pass establishes those contracts, not interactive drag/drop,
Finder dark-mode label legibility, auto-open, display scaling or Gatekeeper trust. The existing
ad-hoc-only/not-notarized policy and unresolved provenance notices remain unchanged.
