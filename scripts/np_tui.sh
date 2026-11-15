#!/bin/zsh
# np_tui.sh — NordPass vault status banner for interactive terminal startup.
# Add to ~/.zshrc:  [[ -o interactive ]] && source ~/YZKR/scripts/np_tui.sh
# Also runnable directly. No-ops for non-interactive shells / non-TTY output.

_np_tui_status() {
  emulate -L zsh
  setopt err_return pipe_fail no_unset

  [[ -o interactive && -t 1 ]] || return 0
  local np_bin="${NP_BIN:-/opt/homebrew/bin/np}"
  [[ -x $np_bin ]] || return 0

  # YZKR obsidian palette (ai-os/theme.json)
  local dim=$'\e[38;2;139;139;153m' ok=$'\e[38;2;124;231;172m' warn=$'\e[38;2;245;196;81m' acc=$'\e[38;2;167;139;250m' rst=$'\e[0m'

  local json authed unlocked email
  json=$("$np_bin" whoami --format json 2>/dev/null) || { print -r -- "${dim}┃ np${rst} ${warn}unreachable${rst}"; return 0; }
  authed=$(jq -r '.authenticated // false' <<<"$json")
  unlocked=$(jq -r '.vaultUnlocked // false' <<<"$json")
  email=$(jq -r '.email // "-"' <<<"$json")

  if [[ $authed != true ]]; then
    print -r -- "${dim}┃ np${rst} ${warn}signed out${rst}  ${dim}→ np signin${rst}"
  elif [[ $unlocked == true ]]; then
    print -r -- "${dim}┃ np${rst} ${ok}● unlocked${rst} ${dim}${email}${rst}"
  else
    print -r -- "${dim}┃ np${rst} ${warn}○ locked${rst} ${dim}${email}${rst}  ${acc}np unlock${rst}"
  fi
}

_np_tui_status
unfunction _np_tui_status
