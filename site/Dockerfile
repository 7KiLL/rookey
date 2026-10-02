# rookey.click: the landing page, the docs and the install one-liners. Build from the repo root, so the
# installers, fonts and docs come from where they live:  docker build -f site/Dockerfile .
FROM node:22-alpine AS docs
WORKDIR /docs
COPY docs/package.json docs/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY docs/ .
RUN npm run build

FROM nginx:stable-alpine
COPY site/nginx.conf /etc/nginx/conf.d/default.conf
COPY site/index.html install.sh install.ps1 /usr/share/nginx/html/
COPY src/ui/icon.svg /usr/share/nginx/html/icon.svg
COPY src/ui/commissioner-latin.woff2 src/ui/martian-mono-latin.woff2 /usr/share/nginx/html/fonts/
COPY --from=docs /docs/.vitepress/dist /usr/share/nginx/html/docs
