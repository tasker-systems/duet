"""Spike: the smallest helper the shared timeline needs.

Serves spikes/shared-timeline/ as static files on http://127.0.0.1:8765/ and adds one route:
  GET  /marks  -> every line of data/marks.jsonl, as a JSON array
  POST /marks  -> appends the posted JSON object as one line, stamped by "owner"

Claude never goes through this: it appends to data/marks.jsonl directly (tools/mark.py).
Standard library only. Usage: python3 -I tools/serve.py [PORT]
"""
import datetime
import http.server
import json
import sys
import uuid
from functools import partial
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MARKS = ROOT / "data" / "marks.jsonl"


def read_marks():
    if not MARKS.exists():
        return []
    out = []
    for n, line in enumerate(MARKS.read_text().splitlines(), 1):
        if line.strip():
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError as e:
                out.append({"id": f"bad-line-{n}", "by": "?", "kind": "error", "note": f"line {n}: {e}"})
    return out


class Handler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_GET(self):
        if self.path.split("?")[0] == "/marks":
            return self._json(200, read_marks())
        return super().do_GET()

    def do_POST(self):
        if self.path != "/marks":
            return self._json(404, {"error": "only /marks takes a POST"})
        # Only the page itself may write: a JSON content type cannot come from a cross-site form
        # without a preflight this server never answers, and a browser's Origin must be ours.
        origin = self.headers.get("Origin")
        host = f"http://{self.headers.get('Host', '')}"
        if not self.headers.get("Content-Type", "").startswith("application/json") or (origin and origin != host):
            return self._json(403, {"error": "only the page served here may write marks"})
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        try:
            raw = json.loads(body)
            assert isinstance(raw, dict) and raw.get("kind") in ("mark", "reply", "edit")
            entry = {"kind": raw["kind"]}
            if raw.get("on") is not None:
                entry["on"] = str(raw["on"])[:64]
            if raw.get("from") is not None:
                entry["from"], entry["to"] = float(raw["from"]), float(raw["to"])
            if raw.get("note"):
                entry["note"] = str(raw["note"])[:4000]
        except Exception:
            return self._json(400, {"error": "expected a JSON object with kind mark, reply or edit"})
        entry["by"] = "owner"
        entry["id"] = "m-" + uuid.uuid4().hex[:8]
        entry["at"] = datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds")
        with MARKS.open("a") as f:
            f.write(json.dumps(entry) + "\n")
        return self._json(200, entry)

    def _json(self, code, obj):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), partial(Handler, directory=str(ROOT)))
    print(f"open http://127.0.0.1:{port}/ in your own browser (Chrome or Safari); marks: {MARKS}")
    server.serve_forever()
