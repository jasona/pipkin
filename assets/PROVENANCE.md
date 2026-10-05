# Bundled asset provenance

| Asset | Source | License |
| --- | --- | --- |
| `fonts/IBMPlexSans-*.ttf` | IBM Plex Sans, now used only for italics (Poppins has no italic here), copied from the Zed checkout at `zed/assets/fonts/ibm-plex-sans` (rev a846890) | SIL Open Font License 1.1 (`fonts/IBMPlexSans-LICENSE.txt`) |
| `fonts/Poppins-{Regular,SemiBold,Bold}.ttf` | Poppins, from the Pipkin website checkout (`../pipkinai.com/assets/fonts`), the brand typeface | SIL Open Font License 1.1 (`fonts/Poppins-OFL.txt`) |
| `fonts/Lilex-*.ttf` | Lilex, copied from `zed/assets/fonts/lilex` (rev a846890) | SIL Open Font License 1.1 (`fonts/Lilex-OFL.txt`) |
| `icons/*.svg` | Lucide icons, `lucide-static@1.51.0` from unpkg.com | ISC (`icons/LICENSE`) |
| `brand/mascot.png` | The Pipkin mascot ("focused" pose), trimmed and scaled from `../pipkinai.com/assets/plates/mascot.png` | Pipkin brand artwork, used as approved; not to be altered or recoloured |
| `brand/mascot-waiting.png` | Copied unchanged from the user-provided `pipkin-waiting.png` (the root-level source was removed after copying); displayed in the Changes pane empty state. The original focused-pose artwork is untouched. | Pipkin brand artwork, supplied for use by the owner |

No other assets are bundled. Zed's own icon set is not used.
