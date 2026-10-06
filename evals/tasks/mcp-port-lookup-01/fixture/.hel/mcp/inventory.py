#!/usr/bin/env python3
"""Minimal stdio MCP server for the H10 MCP task. Standard library only.

Port assignments live only here, under .hel, which hel's model tools cannot read.
Every received message is appended to .hel/mcp/events.jsonl for trace auditing.
"""
import json
import os
import sys

PORTS = {"api": 8437, "worker": 9123, "admin": 8611}
TOOLS = [
    {
        "name": "list_services",
        "description": "List services registered in the inventory.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "lookup_port",
        "description": "Return the port assigned to a service.",
        "inputSchema": {
            "type": "object",
            "properties": {"service": {"type": "string"}},
            "required": ["service"],
        },
    },
]
LOG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "events.jsonl")


def log(message):
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(json.dumps(message, ensure_ascii=False) + "\n")


def text(value, error=False):
    return {"content": [{"type": "text", "text": value}], "isError": error}


def call(name, args):
    if name == "list_services":
        return text("\n".join(sorted(PORTS)))
    if name == "lookup_port":
        service = args.get("service")
        if service in PORTS:
            return text(f"{service}: {PORTS[service]}")
        return text(f"unknown service: {service}", error=True)
    return None


def handle(message):
    method, mid = message.get("method"), message.get("id")
    if mid is None:
        return None
    if method == "initialize":
        version = message.get("params", {}).get("protocolVersion", "2025-11-25")
        result = {
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "inventory", "version": "1.0.0"},
        }
    elif method == "ping":
        result = {}
    elif method == "tools/list":
        result = {"tools": TOOLS}
    elif method == "tools/call":
        params = message.get("params", {})
        result = call(params.get("name"), params.get("arguments") or {})
        if result is None:
            return {"jsonrpc": "2.0", "id": mid,
                    "error": {"code": -32602, "message": "unknown tool"}}
    else:
        return {"jsonrpc": "2.0", "id": mid,
                "error": {"code": -32601, "message": f"method not found: {method}"}}
    return {"jsonrpc": "2.0", "id": mid, "result": result}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        log(message)
        response = handle(message)
        if response is not None:
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
