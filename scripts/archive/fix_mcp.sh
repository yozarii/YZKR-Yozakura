#!/bin/zsh
set -e

YZKR_DIR="$HOME/YZKR"
SCRIPTS_DIR="$YZKR_DIR/scripts"
CONFIG_PATH="$HOME/Library/Application Support/Claude/claude_desktop_config.json"

mkdir -p "$SCRIPTS_DIR"

# 1. Initialize ~/YZKR as a Git repo so mcp-server-git starts cleanly
if [[ ! -d "$YZKR_DIR/.git" ]]; then
  echo "Initializing Git repository in $YZKR_DIR..."
  git -C "$YZKR_DIR" init
fi

# 2. Write a dedicated FastMCP wrapper for NordPass CLI (/opt/homebrew/bin/np)
cat << 'EOF' > "$SCRIPTS_DIR/np_mcp.py"
import subprocess
from mcp.server.fastmcp import FastMCP

mcp = FastMCP("np-manager")
NP_BIN = "/opt/homebrew/bin/np"

def _run_np(args: list[str]) -> str:
    res = subprocess.run([NP_BIN] + args, capture_output=True, text=True)
    if res.returncode != 0:
        err = res.stderr.strip() or res.stdout.strip()
        return f"NordPass CLI error (exit {res.returncode}): {err}"
    return res.stdout.strip()

@mcp.tool()
def vault_status() -> str:
    """Check active NordPass CLI account and vault lock status."""
    return _run_np(["whoami", "--format", "json"])

@mcp.tool()
def list_items(search: str = "", item_type: str = "", limit: int = 25) -> str:
    """List NordPass vault items (metadata/titles/usernames) matching optional search or type (password, note, card)."""
    args = ["item", "list", "--limit", str(limit), "--format", "json"]
    if search:
        args.extend(["--search", search])
    if item_type:
        args.extend(["--type", item_type])
    return _run_np(args)

@mcp.tool()
def get_item(item_id_or_name: str, field: str = "") -> str:
    """Retrieve a specific NordPass item or single field (e.g. username, url, note, password)."""
    args = ["item", "get", item_id_or_name, "--format", "json"]
    if field:
        args.extend(["--field", field])
    return _run_np(args)

@mcp.tool()
def list_folders(search: str = "") -> str:
    """List NordPass vault folders."""
    args = ["folder", "list", "--format", "json"]
    if search:
        args.extend(["--search", search])
    return _run_np(args)

@mcp.tool()
def generate_password(length: int = 20, symbols: bool = True) -> str:
    """Generate a strong random password via NordPass CLI."""
    args = ["generate", "password", "--length", str(length), "--format", "json"]
    if not symbols:
        args.extend(["--symbols", "false"])
    return _run_np(args)

if __name__ == "__main__":
    mcp.run()
EOF

# 3. Patch claude_desktop_config.json cleanly without touching preferences
python3 - << 'EOF'
import json, os

config_path = os.path.expanduser("~/Library/Application Support/Claude/claude_desktop_config.json")
yzkr_dir = os.path.expanduser("~/YZKR")
np_mcp_script = os.path.join(yzkr_dir, "scripts", "np_mcp.py")

with open(config_path, "r", encoding="utf-8") as f:
    config = json.load(f)

config["mcpServers"] = {
    "filesystem": {
        "command": "/opt/homebrew/bin/npx",
        "args": [
            "-y",
            "@modelcontextprotocol/server-filesystem",
            yzkr_dir
        ]
    },
    "git": {
        "command": "/opt/homebrew/bin/uvx",
        "args": [
            "mcp-server-git",
            "--repository",
            yzkr_dir
        ]
    },
    "memory": {
        "command": "/opt/homebrew/bin/npx",
        "args": [
            "-y",
            "@modelcontextprotocol/server-memory"
        ]
    },
    "np-manager": {
        "command": "/opt/homebrew/bin/uv",
        "args": [
            "run",
            "--with",
            "mcp",
            "python",
            np_mcp_script
        ]
    }
}

with open(config_path, "w", encoding="utf-8") as f:
    json.dump(config, f, indent=2)

print("✅ Patched claude_desktop_config.json successfully.")
EOF

echo "All done! Fully quit Claude Desktop (Cmd+Q) and reopen it."