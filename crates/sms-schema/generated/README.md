# Generated schema metadata

`object-registry.json` is a source-free snapshot of metadata extracted from the
Super Mario Sunshine decompilation source. It contains no retail game assets or
decompilation source text. The snapshot includes object/NPC/enemy metadata,
music-to-wave mappings, per-stage audio behavior, and dialogue voice order.

Maintainers refresh it from a clean decompilation revision:

```powershell
cargo schema-bundle --decomp-root ..
cargo schema-bundle --decomp-root .. --check
```

The generator records the decomp revision, a location-independent source
fingerprint, and the schema generator fingerprint. Graffito validates the
artifact format, extractor compatibility, and registry invariants before using
the bundle; the source fingerprint makes independently generated snapshots
directly comparable.
