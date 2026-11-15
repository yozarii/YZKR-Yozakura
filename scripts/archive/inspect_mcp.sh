#!/bin/zsh
echo "=== 1. Binary Paths ==="
for bin in uvx uv npx node np ollama; do
  loc=$(which $bin 2>/dev/null)
  if [[ -n "$loc" ]]; then
    echo "✅ $bin -> $loc"
  else
    echo "❌ $bin -> NOT FOUND in PATH"
  fi
done

echo "\n=== 2. Check ~/.local/bin & ~/.cargo/bin ==="
ls -la ~/.local/bin/uvx ~/.cargo/bin/uvx 2>/dev/null || echo "No uvx in ~/.local/bin or ~/.cargo/bin"

echo "\n=== 3. Check ~/YZKR Git Status ==="
if [[ -d "$HOME/YZKR/.git" ]]; then
  echo "✅ ~/YZKR is a valid Git repository"
else
  echo "⚠️  ~/YZKR is NOT initialized as a Git repository (mcp-server-git will fail without .git)"
fi

echo "\n=== 4. Inspect 'np' CLI Subcommands ==="
if command -v np >/dev/null 2>&1; then
  np --help 2>&1 | head -n 25
else
  echo "np not installed"
fi

echo "\n=== 5. Current Claude Desktop MCP Config ==="
cat "$HOME/Library/Application Support/Claude/claude_desktop_config.json" 2>/dev/null || echo "Config file missing"