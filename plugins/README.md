# Plugin binaries and legacy compatibility files

The current desktop plugin mode uses a separately supplied, compiled UE Tools
Toolkit v1 bundle. Public CI optionally consumes `NTE_TOOLKIT_BUNDLE` and stages:

```text
plugins/ue-tools/
  d3d12.dll
  plugins/NTE_PluginCombat.dll
```

UE Tools and Mod Loader source belong to the private UE Tools workspace. The old
native Mod plugin source has been removed; public CI does not build or package it
or Mod Loader. This does not remove existing files from users' game directories.

`mods-plugin.version`, `nte-mods.enabled`, `nte-mods/*.nte` and `examples/` are
legacy v7 compatibility assets still referenced by Rust code/tests. The example
`query_mod_events.py` requires an external old runtime exposing the v7 named pipe.
These scripts cannot be installed as Toolkit v1 plugins.

See the [legacy reference audit](../docs/LEGACY_MOD_REFERENCES.md) for remaining
runtime callers and the distinction between the old IPC and Toolkit v1.
