#!/usr/bin/env bash
# Publishes definitions/ to the online game database repository
# (github.com/IKryptikI/achievement-hub-games): rebuilds index.json, copies
# the definitions (and the website's game data, site/) into a checkout in
# .db-repo/, commits and pushes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO=IKryptikI/achievement-hub-games
python3 "$ROOT/scripts/build-index.py"
python3 "$ROOT/scripts/build-site-data.py"
if [ ! -d "$ROOT/.db-repo/.git" ]; then
    gh repo clone "$REPO" "$ROOT/.db-repo"
fi
cd "$ROOT/.db-repo"
git pull -q --ff-only || true
mkdir -p definitions
rsync -a --delete --include='*.toml' --exclude='*' "$ROOT/definitions/" definitions/
cp "$ROOT/definitions/index.json" index.json
mkdir -p site
rsync -a --delete --include='*.json' --exclude='.*' --exclude='*' "$ROOT/site-data/" site/
git add -A
if git diff --cached --quiet; then
    echo "game database: nothing changed"
else
    git commit -qm "Update game definitions ($(git diff --cached --name-only | grep -c toml || true) files)"
    git push -q
    echo "game database: published"
fi
