# Bundled licenses and provenance audit

**Status: partial; not release clearance.** Generated inventories and copied license text are evidence, not a
substitute for reviewing missing notices, upstream applicability, or redistribution requirements.

## What packages carry

`usr/share/doc/pipkin/third-party/inventory.json` lists:

- The locked Linux Rust graph reachable from `pipkin-app` through normal/build edges. Cargo workspace feature
  unification can include extra feature dependencies, so this is a conservative inventory, not a minimal binary
  SBOM or proof of which functions were linked.
- Installed npm package roots and engine workspace packages actually in the staged tree. Workspace symlinks are
  resolved inside the engine; embedded `package.json` test fixtures are not treated as installed packages.
- Declared licenses, source identifiers, copied notice-file hashes and explicit review flags. Host cache paths are
  omitted. Ancestor/repository fallback notices are marked for applicability review.

Full package/vendored-component notice files are copied under `third-party/notices`. Pi's root MIT license and
existing font/icon notices remain in their normal package locations. The unmodified registry source for the
MPL-2.0 `option-ext` dependency is shipped under `third-party/sources/option-ext-0.2.0`; its inventory entry points to
that source. This source provision is specific to the current locked version, not a blanket claim about future MPL
changes. Source/update obligations must be rechecked when dependencies change.

For the current graph, the offered Apache-2.0 alternative is selected for Rust `self_cell`, and BSD-3-Clause for
npm `node-forge`. Their original notices are retained. Copying an optional GPL license text alongside an offered
permissive alternative does not mean Pipkin chose the GPL alternative. GPUI crates declare Apache-2.0; Zed's GPL
application crates are not imported merely because the source repository contains them.

## Initial findings (2026-10-06)

The first actual staged-tree inventory found **552 Rust** and **155 npm/workspace** entries. These counts include
conservatively overincluded feature/build dependencies and shipped example workspaces; they are not runtime-only
counts. All reachable Rust crates declare a license expression. Some published packages omit notice files despite
that declaration, and some npm packages omit the metadata license field.

Three npm packages (`buildcheck`, `cpu-features`, `ssh2`) have no metadata license declaration, but their installed
LICENSE files were inspected and contain the MIT grant and Brian White copyright. They remain flagged rather than
silently rewriting upstream metadata. `cpu-features` also carries a vendored component notice, which is retained.

Open findings include:

- Missing distributed notice files in several AWS SDK packages and other npm packages, including the platform
  esbuild binary package; validate applicable upstream notices and retain them with immutable provenance.
- Missing notice files in published Rust crates such as AccessKit, Lyon, profiling, pathfinder and others; use the
  package's published VCS revision to recover applicable upstream notices rather than copying a random current one.
- Confirm applicability of inherited notices for engine/workspace crates and source examples.
- Complete generated-provider-data provenance review (`packaging/engine-model-data.md`), asset provenance, external
  runtime dependency inventory, and any Apache NOTICE/source-offer requirements.

### Immutable notice recovery

`packaging/upstream-notices.json` records supplemental notice inputs with exact upstream commit URLs and SHA-256
hashes. The initial recovery obtained 33 notice files for 20 Rust package versions using the published crates'
`.cargo_vcs_info.json` revisions. Inventory generation verifies those hashes before copying the files. No network
fetch is needed during packaging. Recovered repository notices remain explicitly flagged for applicability review;
downloading a notice is not an automatic legal-clearance decision.

This reduced packages with no copied notice from 32 to 12, while the total review flags remain 59. Remaining
missing-notice cases include five Rust package versions and seven npm package versions. Their provenance and
applicability still require follow-up; do not replace immutable inputs with unreviewed current upstream text.

The inventory's `reviewRequired` entries are the machine-readable follow-up list. Do not mark the release-plan
license/provenance task complete until those findings are resolved or an explicit acceptable review decision is
recorded. Regenerate/review the inventory for every changed candidate; a prior snapshot is not clearance of a new
app/engine pair.

## Reproduction

```sh
scripts/package.sh "$PI_CHECKOUT"  # includes inventory generation from locked metadata and the actual engine
python3 scripts/test-license-inventory.py
```

Python 3 is a build/qualification dependency, not a new installed-app runtime dependency. The fixture test checks
notice retention, workspace-link handling, exclusion of embedded fixture manifests, MPL source provision and
omission of private host paths. No provider credentials or inference requests are involved.
