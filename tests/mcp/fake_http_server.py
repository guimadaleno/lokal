"""A fake MCP server over Streamable HTTP, for Lokal's tests.

    fake_http_server.py MODE LOG

prints `PORT <n>` once it listens on 127.0.0.1, and writes each request it
gets to LOG as a JSON line: the HTTP method, the headers, the body.

    legacy    2025-11-25: a session from initialize, JSON answers
    sse       2025-11-25 too, every answer an event stream that pings the
              client, and waits for its answer, before the result
    expire    as legacy, but it forgets the session after the first call
    modern    2026-07-28: refuses initialize; every request must carry its
              headers (MCP-Protocol-Version, Mcp-Method, Mcp-Name), and the
              tool `where` its region as Mcp-Param-Region
    token     as legacy, but 401 without `Authorization: Bearer secret-token`
    redirect  every POST answered with 307 to another address
    flood     as sse, but a call's stream sends 30 pings at once, and the
              server holds each answer to one for 3 seconds
    hangnote  as legacy, but it holds notifications/initialized for a minute

Tools: echo (gives back its text), argv (whether a curl runs, and whether
any curl's command line holds "secret-token") and slow (an answer only after a minute; over SSE the
stream stays open meanwhile).
"""
import base64
import json
import os
import sys
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODE = sys.argv[1]
LOG = sys.argv[2]
ERA = "2026-07-28" if MODE == "modern" else "2025-11-25"
sessions = set()
forgotten = threading.Event()
pinged = threading.Event()
lock = threading.Lock()

TOOLS = [
    {"name": "echo", "description": "Gives back the text it gets.",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}},
    {"name": "argv", "description": "Whether a command line holds the token.",
     "inputSchema": {"type": "object", "properties": {}}},
    {"name": "slow", "description": "Takes a minute.",
     "inputSchema": {"type": "object", "properties": {}}},
]
if MODE == "modern":
    TOOLS += [
        {"name": "where", "description": "Says the region it got.",
         "inputSchema": {"type": "object", "properties": {
             "region": {"type": "string", "x-mcp-header": "Region"}}}},
        {"name": "broken", "description": "A mark the spec doesn't allow.",
         "inputSchema": {"type": "object", "properties": {
             "n": {"type": "number", "x-mcp-header": "N"}}}},
    ]


def decoded(value):
    """A header value as the 2026-07-28 revision encodes it."""
    if value and value.startswith("=?base64?") and value.endswith("?="):
        return base64.b64decode(value[len("=?base64?"):-2]).decode()
    return value


def token_in_argv():
    """Whether a curl running now has the token on its command line."""
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            with open(f"/proc/{pid}/cmdline", "rb") as cmdline:
                words = cmdline.read().split(b"\0")
        except OSError:
            continue
        if words[0].endswith(b"curl") and any(b"secret-token" in word for word in words):
            return True
    return False


def curls_running():
    """How many curl processes run now: the call's own, at least."""
    count = 0
    for pid in os.listdir("/proc"):
        try:
            with open(f"/proc/{pid}/cmdline", "rb") as cmdline:
                count += cmdline.read().split(b"\0")[0].endswith(b"curl")
        except OSError:
            pass
    return count


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def log(self, body):
        with lock, open(LOG, "a") as log:
            log.write(json.dumps({"method": self.command,
                                  "headers": {k.lower(): v for k, v in self.headers.items()},
                                  "body": body}) + "\n")

    def reply(self, status, body=None, headers=()):
        data = b"" if body is None else json.dumps(body).encode()
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        if body is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_DELETE(self):
        self.log(None)
        sessions.discard(self.headers.get("Mcp-Session-Id"))
        self.reply(200)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        message = json.loads(self.rfile.read(length) or b"{}")
        self.log(message)
        if MODE == "redirect":
            return self.reply(307, None, [("Location", "https://elsewhere.example/mcp")])
        if MODE == "token" and self.headers.get("Authorization") != "Bearer secret-token":
            return self.reply(401, None, [("WWW-Authenticate", 'Bearer realm="test"')])
        method = message.get("method")
        id = message.get("id")
        if id is None or method is None:
            # A notification, or the client's answer to a ping.
            if method is None and message.get("result") == {}:
                pinged.set()
                if MODE == "flood":
                    time.sleep(3)
            if MODE == "hangnote" and method == "notifications/initialized":
                time.sleep(60)
            return self.reply(202)
        if MODE == "modern":
            return self.modern(method, id, message.get("params") or {})
        if method == "initialize":
            session = uuid.uuid4().hex
            sessions.add(session)
            result = {"protocolVersion": ERA, "capabilities": {"tools": {}},
                      "serverInfo": {"name": "fake", "version": "1"}}
            return self.answer(id, result, [("Mcp-Session-Id", session)])
        session = self.headers.get("Mcp-Session-Id")
        if session not in sessions:
            return self.reply(404, {"jsonrpc": "2.0", "id": None,
                                    "error": {"code": -32001, "message": "Session not found"}})
        if self.headers.get("MCP-Protocol-Version") != ERA:
            return self.reply(400, None)
        self.legacy(method, id, message.get("params") or {}, session)

    def legacy(self, method, id, params, session):
        if method == "tools/list":
            return self.answer(id, {"tools": TOOLS})
        if method == "tools/call":
            if MODE == "expire":
                if forgotten.is_set():
                    pass
                else:
                    forgotten.set()
                    sessions.discard(session)
            return self.call(id, params)
        self.reply(200, {"jsonrpc": "2.0", "id": id,
                         "error": {"code": -32601, "message": "Method not found"}})

    def modern(self, method, id, params):
        def mismatch(why):
            self.reply(400, {"jsonrpc": "2.0", "id": id,
                             "error": {"code": -32020, "message": f"HeaderMismatch: {why}"}})
        meta = params.get("_meta") or {}
        if self.headers.get("MCP-Protocol-Version") != ERA or \
                meta.get("io.modelcontextprotocol/protocolVersion") != ERA:
            return self.reply(400, {"jsonrpc": "2.0", "id": id, "error": {
                "code": -32022, "message": "Unsupported protocol version",
                "data": {"supported": [ERA]}}})
        if self.headers.get("Mcp-Method") != method:
            return mismatch("Mcp-Method")
        if method == "server/discover":
            return self.answer(id, {"supportedVersions": [ERA], "capabilities": {"tools": {}},
                                    "serverInfo": {"name": "fake", "version": "1"}})
        if method == "tools/list":
            return self.answer(id, {"tools": TOOLS})
        if method == "tools/call":
            if decoded(self.headers.get("Mcp-Name")) != params.get("name"):
                return mismatch("Mcp-Name")
            if params.get("name") == "where":
                region = (params.get("arguments") or {}).get("region")
                if decoded(self.headers.get("Mcp-Param-Region")) != region:
                    return mismatch("Mcp-Param-Region")
                return self.answer(id, text_result(f"region {region}"))
            return self.call(id, params)
        self.reply(404, {"jsonrpc": "2.0", "id": id,
                         "error": {"code": -32601, "message": "Method not found"}})

    def call(self, id, params):
        name = params.get("name")
        arguments = params.get("arguments") or {}
        if name == "echo":
            return self.answer(id, text_result(arguments.get("text", "")))
        if name == "argv":
            return self.answer(id, text_result(
                f"curls: {curls_running() > 0}, token in argv: {token_in_argv()}"))
        if name == "slow":
            if MODE == "sse":
                return self.stream_until_gone(id)
            time.sleep(60)
            return self.answer(id, text_result("done"))
        self.answer(id, {"content": [{"type": "text", "text": f"Unknown tool: {name}"}],
                         "isError": True})

    def answer(self, id, result, headers=()):
        if MODE == "modern":
            result = dict(result, resultType="complete")
        message = {"jsonrpc": "2.0", "id": id, "result": result}
        if MODE == "flood" and "content" in result:
            return self.flood(message)
        if MODE != "sse":
            return self.reply(200, message, headers)
        # A priming event, a ping the client must answer, a notification,
        # then the answer.
        self.send_response(200)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        pinged.clear()
        self.event('id: 1\ndata:\n\n')
        self.event('event: message\ndata: {"jsonrpc": "2.0", "id": "ping-1",\n'
                   'data:  "method": "ping"}\n\n')
        pinged.wait(5)
        self.event(': a comment\ndata: {"jsonrpc": "2.0", "method": "notifications/message",'
                   ' "params": {"level": "info", "data": "working"}}\n\n')
        self.event("data: " + json.dumps(message) + "\n\n")
        self.close_connection = True

    def flood(self, message):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        for n in range(30):
            self.event(f'data: {{"jsonrpc": "2.0", "id": "ping-{n}", "method": "ping"}}\n\n')
        self.event("data: " + json.dumps(message) + "\n\n")
        self.close_connection = True

    def stream_until_gone(self, id):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        started = time.time()
        try:
            while time.time() - started < 60:
                self.event(": still working\n\n")
                time.sleep(0.1)
        except OSError:
            with lock, open(LOG, "a") as log:
                log.write(json.dumps({"method": "GONE"}) + "\n")
        self.close_connection = True

    def event(self, text):
        self.wfile.write(text.encode())
        self.wfile.flush()


def text_result(text):
    return {"content": [{"type": "text", "text": text}]}


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(f"PORT {server.server_address[1]}", flush=True)
server.serve_forever()
