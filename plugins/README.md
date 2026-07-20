# Runtime plugins

`nte-dps-tool.exe` reads optional runtime plugins from this directory. The
equipment plugin must be named `dwmapi.dll`:

```text
plugins/
  dwmapi.dll
```

Building `native/nte-equipment-plugin/nte-equipment-plugin.sln` stages the
compiled DLL here automatically. Windows GUI release archives include this
directory beside `nte-dps-tool.exe`.
