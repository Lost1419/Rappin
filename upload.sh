#!/usr/bin/env bash
set -e

# Script to push Rappin to GitHub
echo "==> Pushing Rappin to https://github.com/Lost1419/Rappin..."

# Ensure remote is origin
git remote set-url origin https://github.com/Lost1419/Rappin.git

# Push main branch
git push -u origin main

echo "==> Upload completed successfully!"
