"""A fake MCP server over stdio, for Lokal's tests.

    fake_server.py legacy   starts with initialize (2025-06-18), and pings
                            the client once it's initialized
    fake_server.py modern   stateless (2026-07-28): refuses initialize, and
                            any request without the version in _meta
    fake_server.py crash    writes a reason to stderr and ends

Tools, one a page: echo (gives back its text), picture (a 2x2 PNG) and
slow (answers only when cancelled, or after a minute).
"""
import base64
import json
import sys
import time

ERA = sys.argv[1]
VERSION_KEY = "io.modelcontextprotocol/protocolVersion"
PNG = base64.b64encode(
    bytes.fromhex(
        "89504e470d0a1a0a0000000d494844520000000200000002010300000048789f67"
        "00000003504c54453366cc202224a90000000c4944415408d76360606000000004"
        "00012734270a0000000049454e44ae426082"
    )
).decode()
TOOLS = [
    {"name": "echo", "description": "Gives back the text it gets.",
     "inputSchema": {"type": "object", "properties": {
         "text": {"type": "string", "description": "What to give back."}},
         "required": ["text"]}},
    {"name": "picture", "description": "A small picture.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "slow", "description": "Takes a minute.",
     "inputSchema": {"type": "object", "properties": {}}},
]


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def result(id, value):
    if ERA == "modern":
        value = dict(value, resultType="complete")
    send({"jsonrpc": "2.0", "id": id, "result": value})


def error(id, code, message):
    send({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})


def call(id, params):
    name = params.get("name")
    arguments = params.get("arguments") or {}
    if name == "echo":
        result(id, {"content": [{"type": "text", "text": arguments.get("text", "")}]})
    elif name == "picture":
        result(id, {"content": [{"type": "image", "data": PNG, "mimeType": "image/png"}]})
    elif name == "slow":
        for line in sys.stdin:
            message = json.loads(line)
            if message.get("method") == "notifications/cancelled":
                return
        time.sleep(60)
        result(id, {"content": [{"type": "text", "text": "done"}]})
    else:
        error(id, -32602, "Unknown tool: " + str(name))


def main():
    if ERA == "crash":
        sys.stderr.write("fake server: missing token\n")
        sys.exit(1)
    sys.stderr.write("fake server: started\n")
    initialized = False
    for line in sys.stdin:
        message = json.loads(line)
        method, id = message.get("method"), message.get("id")
        params = message.get("params") or {}
        if method is None:
            continue
        if ERA == "legacy":
            if method == "initialize":
                result(id, {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                            "serverInfo": {"name": "fake", "version": "1"}})
                continue
            if method == "notifications/initialized":
                initialized = True
                send({"jsonrpc": "2.0", "id": "ping-1", "method": "ping"})
                continue
            if not initialized:
                continue
        else:
            if method == "initialize":
                error(id, -32601, "Method not found")
                continue
            if id is not None and params.get("_meta", {}).get(VERSION_KEY) != "2026-07-28":
                error(id, -32602, "Missing protocol version")
                continue
            if method == "server/discover":
                result(id, {"supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}},
                            "ttlMs": 0, "cacheScope": "private"})
                continue
        if method == "tools/list":
            page = int(params.get("cursor") or 0)
            listing = {"tools": [TOOLS[page]]}
            if page + 1 < len(TOOLS):
                listing["nextCursor"] = str(page + 1)
            if ERA == "modern":
                listing.update(ttlMs=0, cacheScope="private")
            result(id, listing)
        elif method == "tools/call":
            call(id, params)
        elif id is not None:
            error(id, -32601, "Method not found")


main()
