# 部署 dps.o-na-ni.com（VPS）

## 1. DNS

在 o-na-ni.com 的域名解析里加一条记录：

| 类型 | 主机记录 | 值          |
|------|----------|-------------|
| A    | dps      | 你的 VPS IP |

等待解析生效（一般几分钟到几十分钟，`ping dps.o-na-ni.com` 看是否返回 VPS IP）。

## 2. 上传站点文件

把仓库里的 `site/` 目录整个传到 VPS 上：

```bash
scp -r site/* youruser@your-vps-ip:/tmp/dps-site
ssh youruser@your-vps-ip
sudo mkdir -p /var/www/dps.o-na-ni.com
sudo cp -r /tmp/dps-site/* /var/www/dps.o-na-ni.com/
sudo chown -R www-data:www-data /var/www/dps.o-na-ni.com   # 按你的 nginx 运行用户调整
```

## 3. Nginx

把 `deploy/nginx-dps.o-na-ni.com.conf` 传到 VPS，放进 nginx 配置目录（按你现有的目录结构选其一）：

```bash
sudo cp nginx-dps.o-na-ni.com.conf /etc/nginx/sites-available/dps.o-na-ni.com
sudo ln -s /etc/nginx/sites-available/dps.o-na-ni.com /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

因为这个 server block 是按 `server_name dps.o-na-ni.com` 单独匹配的，不会影响你现有跑在 o-na-ni.com 主域名上的站点。

## 4. HTTPS

```bash
sudo certbot --nginx -d dps.o-na-ni.com
```

certbot 会自动签发证书并改写 nginx 配置加上 443 server block + HTTP→HTTPS 跳转。

## 5. 验证落地页可访问

浏览器打开 `https://dps.o-na-ni.com/`，确认页面、截图、语言切换按钮都正常。

## 6. Google Search Console 验证

1. 打开 [Search Console](https://search.google.com/search-console)，添加资源，URL 前缀填 `https://dps.o-na-ni.com/`。
2. 用 **HTML 文件** 方式验证：下载 Google 给的验证文件，放进 `/var/www/dps.o-na-ni.com/` 目录（和 `index.html` 同级），不要改名，然后点验证。
   - 也可以用 **域名提供商（DNS）** 方式：因为你能控制 o-na-ni.com 的 DNS，加一条 TXT 记录即可，覆盖范围更大（连带验证整个 o-na-ni.com 及所有子域名）。
3. 验证通过后，用"网址检查"工具提交 `https://dps.o-na-ni.com/` 请求编入索引。
4. 在"健全性"→ Sitemap 里提交 `https://dps.o-na-ni.com/sitemap.xml`。

## 7. 回链

记得在以下地方加上 `https://dps.o-na-ni.com/` 的链接，帮助搜索引擎更快发现并抓取：

- GitHub 仓库描述 / README 顶部
- B 站视频简介
- 任何你发过项目介绍的论坛、QQ群公告、Discord 等
