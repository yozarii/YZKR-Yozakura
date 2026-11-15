#!/bin/bash
# Installer entry point for Zen PWA v5 (castLabs Electron + Widevine).
# All of the real work lives in the repository so that updates stay in one place.
set -e
exec "$HOME/YZKR/scripts/ZenPWA/build.sh" "$@"
