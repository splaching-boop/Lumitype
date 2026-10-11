#!/usr/bin/env bash
# Aligne les métadonnées du projet sur la licence MIT (Cargo.toml racine + package.json).
set -euo pipefail
cd "$(dirname "$0")"
sed -i 's/^license = "MIT OR Apache-2.0"/license = "MIT"/' Cargo.toml
grep -q '"license"' package.json || sed -i '0,/"version": *"[^"]*",/s//&\n  "license": "MIT",/' package.json
grep -n '^license' Cargo.toml; grep -n '"license"' package.json
