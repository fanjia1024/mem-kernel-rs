import hashlib
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

DIM = 64


def embed_text(text: str):
    seed = hashlib.sha256(text.encode("utf-8")).digest()
    values = []
    for i in range(DIM):
        b = seed[i % len(seed)]
        values.append((b / 255.0) * 2.0 - 1.0)
    return values


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/health":
            body = b"ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        if self.path != "/v1/embeddings":
            self.send_response(404)
            self.end_headers()
            return

        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        try:
            req = json.loads(raw.decode("utf-8"))
        except Exception:
            self.send_response(400)
            self.end_headers()
            return

        inputs = req.get("input", "")
        if isinstance(inputs, str):
            inputs = [inputs]
        if not isinstance(inputs, list):
            self.send_response(400)
            self.end_headers()
            return

        data = []
        tokens = 0
        for idx, text in enumerate(inputs):
            s = str(text)
            tokens += len(s)
            data.append({"object": "embedding", "embedding": embed_text(s), "index": idx})

        resp = {
            "object": "list",
            "data": data,
            "model": req.get("model", "mock-embedding"),
            "usage": {"prompt_tokens": tokens, "total_tokens": tokens},
        }
        body = json.dumps(resp).encode("utf-8")

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    server = HTTPServer(("0.0.0.0", 8088), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
