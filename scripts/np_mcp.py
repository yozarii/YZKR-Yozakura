# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "mcp>=2,<3",
# ]
# ///

import subprocess
from mcp.server import MCPServer

mcp = MCPServer("np-manager")
NP_BIN = "/opt/homebrew/bin/np"

def _run_np(args: list[str]) -> str:
    res = subprocess.run([NP_BIN, *args], capture_output=True, text=True, timeout=30)
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
