# rookey.click: the landing page and the install one-liners. Build from the repo root, so the
# installers and fonts come from where they live:  docker build -f site/Dockerfile .
FROM nginx:stable-alpine
COPY site/nginx.conf /etc/nginx/conf.d/default.conf
COPY site/index.html install.sh install.ps1 /usr/share/nginx/html/
COPY src/ui/icon.svg /usr/share/nginx/html/icon.svg
COPY src/ui/commissioner-latin.woff2 src/ui/martian-mono-latin.woff2 /usr/share/nginx/html/fonts/
