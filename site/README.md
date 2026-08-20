# 静态站点构建

`site/` 只保存 HTML、robots 和 sitemap 源文件。截图以仓库根目录的
`images/CN|EN/` 为唯一事实源，站点图标以 `res/icons/app-icon.png` 为唯一事实源；
不要把这些图片的副本重新提交到 `site/assets/img/`。

发布前从仓库根目录构建一个全新的部署目录：

```powershell
pwsh -NoProfile -File scripts/build_site.ps1 -OutputDirectory target/site
```

构建命令拒绝覆盖已有目录。请把生成目录（默认 `target/site/`）中的内容整体部署，
不要直接部署源目录 `site/`。

提交前可运行完整性回归：

```powershell
pwsh -NoProfile -File scripts/test_site_build.ps1
```

该测试会在临时目录重建站点，逐个比较 13 个生成图片与 canonical source 的
SHA-256，并确认仓库不再跟踪 `site/assets/img/` 副本。
