#!/bin/sh
set -eu
cruise_checkout=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if command -v cargo >/dev/null 2>&1; then
  cargo install --locked --path "$cruise_checkout" --root "$HOME/.local" --force
else
  "$HOME/.cargo/bin/cargo" install --locked --path "$cruise_checkout" --root "$HOME/.local" --force
fi
printf '%s\n' 'Installed cruise. Ensure ~/.local/bin is on PATH, then run cruise run.'
