#!/bin/zsh
# name: YZKR Workspace
# icon: 🧭
# description: Start Ollama service and open YZKR folder and HANDOFF.md file
# yzkr-steps: [{"type": "shell", "value": "brew services start ollama"}, {"type": "open_path", "value": "~/YZKR"}, {"type": "open_path", "value": "~/YZKR/ai-memory/HANDOFF.md"}]
set -euo pipefail
export PATH="/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:$PATH"  # launchers run with a bare PATH

brew services start ollama
open /Users/yozari/YZKR
open /Users/yozari/YZKR/ai-memory/HANDOFF.md
