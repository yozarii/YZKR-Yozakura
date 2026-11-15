import json
import os
import subprocess

def run_cmd(cmd):
    print(f"Executing: {' '.join(cmd)}")
    subprocess.run(cmd, check=True)

print("=== Initializing YZKR Ecosystem Setup ==-\\n")

# 1. Install Sol Launcher & Rectangle Window Manager via Homebrew
try:
    run_cmd(["brew", "install", "--cask", "sol"])
    run_cmd(["brew", "install", "--cask", "rectangle"])
except Exception as e:
    print(f"Homebrew cask install note: {e}")

# 2. Setup Ollama & Pull Local Models
try:
    run_cmd(["brew", "install", "ollama"])
    # Start ollama background service or remind user
    print("Pulling lightweight SLM (llama3.2:3b) and TypeSafe backend model...")
    subprocess.run(["ollama", "pull", "llama3.2:3b"], check=False)
    subprocess.run(["ollama", "pull", "qwen2.5:3b-instruct-q4_K_M"], check=False) # Used for structured TypeSafe JSON parsing
except Exception as e:
    print(f"Ollama setup note: {e}")

# 3. Configure Claude Desktop JSON Config with MCP Servers
config_dir = os.path.expanduser("~/Library/Application Support/Claude")
config_path = os.path.join(config_dir, "claude_desktop_config.json")
os.makedirs(config_dir, exist_ok=True)

config = {}
if os.path.exists(config_path):
    try:
        with open(config_path, "r", encoding="utf-8") as f:
            config = json.load(f)
    except json.JSONDecodeError:
        config = {}

if "mcpServers" not in config:
    config["mcpServers"] = {}

# Inject MCP servers: Filesystem, Git, Memory, and local np account handler
yzkr_path = os.path.expanduser("~/YZKR")
os.makedirs(yzkr_path, exist_ok=True)

config["mcpServers"].update({
    "filesystem": {
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", yzkr_path]
    },
    "git": {
        "command": "uvx",
        "args": ["mcp-server-git", yzkr_path]
    },
    "memory": {
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-memory"]
    },
    "np-manager": {
        "command": "np",
        "args": ["mcp", "serve"]
    }
})

with open(config_path, "w", encoding="utf-8") as f:
    json.dump(config, f, indent=2)

print(f"\\nSuccessfully injected MCP configuration into: {config_path}")
print("=== YZKR Ecosystem Configuration Complete ===")