#!/usr/bin/env sh
set -eu

COMPOSE_BIN="${COMPOSE_BIN:-docker-compose}"
BOT_URL="${BOT_URL:-http://127.0.0.1:3001}"

run() {
  printf '\n$ %s\n' "$*"
  sh -c "$*" || true
}

run "$COMPOSE_BIN ps"
run "docker inspect wx-ilink-bot --format 'network_mode={{.HostConfig.NetworkMode}} dns={{json .HostConfig.Dns}} networks={{json .NetworkSettings.Networks}}'"
run "docker inspect xhs-downloader --format 'network_mode={{.HostConfig.NetworkMode}} dns={{json .HostConfig.Dns}} networks={{json .NetworkSettings.Networks}}'"
run "docker exec wx-ilink-bot cat /etc/resolv.conf"
run "docker exec xhs-downloader cat /etc/resolv.conf"
run "docker exec wx-ilink-bot getent hosts ilinkai.weixin.qq.com"
run "docker exec wx-ilink-bot getent hosts index.crates.io"
run "docker exec xhs-downloader getent hosts xhslink.com"
run "curl -sS $BOT_URL/health"
run "curl -sS $BOT_URL/xhs/health"
