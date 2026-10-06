# Pinned generated model data

`pi-model-data-<full engine revision>.tar.zst` is the generated provider JSON snapshot paired with
`pi-engine-revision`. Its adjacent `.sha256` file verifies the archive. Release staging restores this input and
runs the pinned Pi `check-model-data.ts` validator. A clean checkout no longer needs a live model-catalog fetch
or ignored assets from a developer checkout. Development overrides still validate the developer's own data.

The initial snapshot was captured from the Pi checkout at
`d2a311097cbcf669e699479587332ae3988a49d0`. It contains the same generated runtime model metadata used in prior
qualified packages; it is not user sessions, credentials, or configuration. Its generated `.manifest.json` hash is
`b92d631bb8bb2cac6f824a03a814baad9452164c518426b8bc4bbc5c2142bb05`.
Pi's MIT license is included in staged engine sources. The broader bundled dependency/data provenance and license
audit remains a release gate; this snapshot does not close it. Model pricing is metadata, not a billing guarantee.

When updating the engine pin:

1. Use a clean checkout of the proposed full revision with locked dependencies installed.
2. Generate/hydrate its model data with Pi's documented tools, then run its model-data validator. This regeneration
   may access public model catalogs; it is an intentional input refresh, not a release-build step.
3. Review the generated JSON and source/provenance changes; include only the required provider JSON and manifest.
4. Create and checksum the new snapshot (from the Pipkin repository root):

   ```sh
   pin=$(cat packaging/pi-engine-revision)
   tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner --zstd \
     -cf "packaging/pi-model-data-$pin.tar.zst" -C "$PI_CHECKOUT/packages/ai/src/providers/data" .
   (cd packaging && sha256sum "pi-model-data-$pin.tar.zst" > "pi-model-data-$pin.tar.zst.sha256")
   ```

5. Run `scripts/test-engine-source.sh`, clean-checkout provisioning, and full packaged-engine qualification.
   Record the new source/data pair and resulting artifact. Do not silently replace an old qualified snapshot;
   qualification belongs to the new pair. Remove obsolete tracked input snapshots when the pin changes.
