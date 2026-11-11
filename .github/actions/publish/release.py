#!/usr/bin/env python3
"""Makes and checks the one manifest every app updates from.

    release.py sign   --app kent --version 0.6.0 --dir out [--public-key update.pub]
    release.py newer  --app kent --version 0.6.0
    release.py verify --app kent (FILE | --url) [--public-key update.pub] [--fetch]

`sign` reads the release laid out in --dir (files/, downloads/, notes/) and
writes --dir/latest.json, signed with UPDATE_KEY from the environment: an
Ed25519 private key in PEM, or its 32-byte seed as 64 hex characters (the form
vovo-release keeps). `newer` refuses a version that is not after the one
already published. `verify` opens a manifest the way an installed copy does,
and with --fetch also downloads every file it names and checks it.

The rules are README.md's; this file is where the publishing side enforces
them. Nothing here talks to the bucket: uploading is the action's job.
"""

import argparse
import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

BASE = "https://download.guimadaleno.com"
APP = re.compile(r"[a-z][a-z0-9]{0,31}")
NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}")
HEX64 = re.compile(r"[0-9a-f]{64}")
# The fixed DER headers around a raw Ed25519 key, so openssl can be handed the
# 32 bytes the apps and update.pub carry.
PKCS8_PREFIX = bytes.fromhex("302e020100300506032b657004220420")
SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")
AGENT = "guimadaleno-release"


class Refused(Exception):
    pass


def die(message: str) -> None:
    print(f"::error::{message}", file=sys.stderr)
    raise SystemExit(1)


# ------------------------------------------------------------------ versions

def parse_version(v: str):
    """`1.2.3` or `1.2.3-beta.4`: three numbers, and an optional pre-release
    of dotted letters and digits. The same reading every client does."""
    core, _, pre = v.partition("-")
    parts = core.split(".")
    if len(parts) != 3 or not all(p.isascii() and p.isdigit() for p in parts):
        return None
    if "-" in v and not (pre and all(i and i.isascii() and i.isalnum() for i in pre.split("."))):
        return None
    return tuple(int(p) for p in parts), (pre if "-" in v else None)


def _pre_order(a: str, b: str) -> int:
    for p, q in zip(a.split("."), b.split(".")):
        if p == q:
            continue
        if p.isdigit() and q.isdigit():
            return (int(p) > int(q)) - (int(p) < int(q))
        if p.isdigit() != q.isdigit():
            return -1 if p.isdigit() else 1
        return (p > q) - (p < q)
    la, lb = len(a.split(".")), len(b.split("."))
    return (la > lb) - (la < lb)


def newer(candidate: str, current: str) -> bool:
    """A release after its own pre-releases; `alpha.10` after `alpha.9`."""
    a, b = parse_version(candidate), parse_version(current)
    if a is None or b is None:
        return False
    if a[0] != b[0]:
        return a[0] > b[0]
    if a[1] is None or b[1] is None:
        return a[1] is None and b[1] is not None
    return _pre_order(a[1], b[1]) > 0


# ---------------------------------------------------------------------- keys

def read_public(path: str) -> bytes:
    text = Path(path).read_text().strip().lower()
    if not HEX64.fullmatch(text):
        die(f"{path} is not 64 hex characters")
    key = bytes.fromhex(text)
    if key == bytes(32):
        die(f"{path} is all zeroes: generate the key first (tools/keygen.sh)")
    return key


def _pem_private(secret: str) -> str:
    s = secret.strip()
    if s.startswith("-----BEGIN"):
        return s + "\n"
    if HEX64.fullmatch(s.lower()):
        der = base64.b64encode(PKCS8_PREFIX + bytes.fromhex(s)).decode()
        return f"-----BEGIN PRIVATE KEY-----\n{der}\n-----END PRIVATE KEY-----\n"
    die("UPDATE_KEY is neither an Ed25519 key in PEM nor 64 hex characters")
    return ""


def _openssl(args: list, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(["openssl", *args], capture_output=True, **kw)


def public_of(secret: str) -> bytes:
    with tempfile.TemporaryDirectory() as tmp:
        key = Path(tmp) / "key.pem"
        key.touch(mode=0o600)
        key.write_text(_pem_private(secret))
        out = _openssl(["pkey", "-in", str(key), "-pubout", "-outform", "DER"])
    if out.returncode != 0 or len(out.stdout) != 44:
        die("UPDATE_KEY is not an Ed25519 key")
    return out.stdout[-32:]


def sign_bytes(message: bytes, secret: str) -> bytes:
    with tempfile.TemporaryDirectory() as tmp:
        key, msg, sig = (Path(tmp) / n for n in ("key.pem", "msg", "sig"))
        key.touch(mode=0o600)
        key.write_text(_pem_private(secret))
        msg.write_bytes(message)
        out = _openssl(["pkeyutl", "-sign", "-inkey", str(key), "-rawin",
                        "-in", str(msg), "-out", str(sig)])
        if out.returncode != 0:
            die("openssl could not sign: " + out.stderr.decode(errors="replace"))
        signature = sig.read_bytes()
    if len(signature) != 64:
        die(f"the signature is {len(signature)} bytes, not 64: UPDATE_KEY is not Ed25519")
    return signature


def verify_bytes(message: bytes, signature: bytes, public: bytes) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        pem, msg, sig = (Path(tmp) / n for n in ("pub.pem", "msg", "sig"))
        der = base64.b64encode(SPKI_PREFIX + public).decode()
        pem.write_text(f"-----BEGIN PUBLIC KEY-----\n{der}\n-----END PUBLIC KEY-----\n")
        msg.write_bytes(message)
        sig.write_bytes(signature)
        out = _openssl(["pkeyutl", "-verify", "-pubin", "-inkey", str(pem), "-rawin",
                        "-in", str(msg), "-sigfile", str(sig)])
    return out.returncode == 0


# ------------------------------------------------------------------ manifest

def _entry_ok(e, app: str, version: str) -> bool:
    return (
        isinstance(e, dict)
        and isinstance(e.get("name"), str) and NAME.fullmatch(e["name"]) is not None
        and e.get("url") == f"{BASE}/{app}/{version}/{e['name']}"
        and isinstance(e.get("size"), int) and e["size"] > 0
        and isinstance(e.get("sha256"), str) and HEX64.fullmatch(e["sha256"]) is not None
    )


def open_manifest(text: str, app: str, public: bytes) -> dict:
    """What an installed copy does with a latest.json, plus the publishing
    side's stricter rule that every URL is exactly where the layout puts it."""
    try:
        envelope = json.loads(text)
        release_text = envelope["release"]
        signature = bytes.fromhex(envelope["signature"])
    except (ValueError, KeyError, TypeError):
        raise Refused("malformed: not a signed release")
    if not isinstance(release_text, str) or len(signature) != 64:
        raise Refused("malformed: not a signed release")
    if not verify_bytes(release_text.encode(), signature, public):
        raise Refused("signature: not this app's release key")
    try:
        release = json.loads(release_text)
    except ValueError:
        raise Refused("malformed: the signed release is not JSON")
    if not isinstance(release, dict):
        raise Refused("malformed: the signed release is not an object")
    if release.get("app") != app:
        raise Refused(f"app: signed for {release.get('app')!r}, not {app!r}")
    version = release.get("version")
    if not isinstance(version, str) or parse_version(version) is None:
        raise Refused(f"contents: version {version!r}")
    files = release.get("files")
    downloads = release.get("downloads", [])
    if not isinstance(files, list) or not files or not isinstance(downloads, list):
        raise Refused("contents: no files")
    for e in files + downloads:
        if not _entry_ok(e, app, version):
            raise Refused(f"contents: {json.dumps(e)[:120]}")
    names = [e["name"] for e in files + downloads]
    if len(set(names)) != len(names):
        raise Refused("contents: a name appears twice")
    return release


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def _entries(folder: Path, app: str, version: str) -> list:
    if not folder.is_dir():
        return []
    out = []
    for path in sorted(folder.iterdir()):
        if not path.is_file():
            die(f"{path} is not a file: a release is a flat list of files")
        if not NAME.fullmatch(path.name):
            die(f"{path.name} is not a name a release may carry")
        if path.stat().st_size == 0:
            die(f"{path} is empty")
        out.append({
            "name": path.name,
            "url": f"{BASE}/{app}/{version}/{path.name}",
            "size": path.stat().st_size,
            "sha256": _sha256(path),
        })
    return out


def write_sums(folder: Path) -> None:
    """downloads/SHA256SUMS: every file of the release, for a person checking
    a download by hand with Get-FileHash or sha256sum. The manifest is for
    programs; this is the same fact for people, and it is a download like the
    others, so the manifest carries its hash too."""
    paths = [p for sub in ("files", "downloads") if (folder / sub).is_dir()
             for p in sorted((folder / sub).iterdir())
             if p.is_file() and p.name != "SHA256SUMS"]
    (folder / "downloads").mkdir(exist_ok=True)
    lines = "".join(f"{_sha256(p)}  {p.name}\n" for p in paths)
    (folder / "downloads" / "SHA256SUMS").write_text(lines)


def build(app: str, version: str, folder: Path, published: int) -> dict:
    if not APP.fullmatch(app):
        die(f"{app!r} is not an app name: lowercase letters and digits, starting with a letter")
    if parse_version(version) is None:
        die(f"{version!r} is not a version (1.2.3, or 1.2.3-beta.4)")
    files = _entries(folder / "files", app, version)
    if not files:
        die(f"{folder}/files is empty: a release has at least the executable it installs")
    downloads = _entries(folder / "downloads", app, version)
    notes = {}
    for lang in ("en", "pt"):
        path = folder / "notes" / f"{lang}.txt"
        notes[lang] = path.read_text(encoding="utf-8").strip() if path.is_file() else ""
    return {
        "app": app,
        "version": version,
        "published": published,
        "files": files,
        "downloads": downloads,
        "notes": notes,
    }


def seal(release: dict, secret: str) -> str:
    text = json.dumps(release, indent=2, ensure_ascii=False)
    signature = sign_bytes(text.encode(), secret)
    envelope = {"release": text, "signature": signature.hex()}
    return json.dumps(envelope, indent=2, ensure_ascii=False) + "\n"


# ------------------------------------------------------------------- network

def fetch(url: str):
    """The body, or None for a 404. Anything else is a failure to report."""
    request = urllib.request.Request(url, headers={"User-Agent": AGENT, "Cache-Control": "no-cache"})
    try:
        with urllib.request.urlopen(request, timeout=60) as r:
            return r.read()
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return None
        die(f"{url}: HTTP {e.code}")
    except urllib.error.URLError as e:
        die(f"{url}: {e.reason}")


# ------------------------------------------------------------------ commands

def cmd_sign(a) -> int:
    public = read_public(a.public_key)
    secret = os.environ.get("UPDATE_KEY", "")
    if not secret.strip():
        die("UPDATE_KEY is not set: no signature, and no installed copy would take the release")
    if public_of(secret) != public:
        die(f"UPDATE_KEY is not the key whose public half is in {a.public_key}: "
            "every installed copy would refuse this release")
    folder = Path(a.dir)
    if not any((folder / "files").glob("*")):
        die(f"{folder}/files is empty: a release has at least the executable it installs")
    write_sums(folder)
    release = build(a.app, a.version, folder, int(time.time() * 1000))
    text = seal(release, secret)
    try:
        open_manifest(text, a.app, public)
    except Refused as e:
        die(f"what was just signed does not open: {e}")
    (folder / "latest.json").write_text(text, encoding="utf-8")
    print(f"{folder}/latest.json: {a.app} {a.version}, "
          f"{len(release['files'])} file(s), {len(release['downloads'])} download(s)")
    return 0


def cmd_newer(a) -> int:
    if not APP.fullmatch(a.app) or parse_version(a.version) is None:
        die(f"{a.app!r} {a.version!r} is not an app name and a version")
    url = f"{BASE}/{a.app}/latest.json"
    body = fetch(url)
    if body is None:
        print(f"{url} does not exist yet: {a.version} is the first release")
        return 0
    try:
        published = json.loads(json.loads(body)["release"])["version"]
    except (ValueError, KeyError, TypeError):
        die(f"{url} is not a release this tool can read; refusing to publish over it")
    if not newer(a.version, published):
        die(f"{a.version} is not after {published}, which is already published: "
            "a published version is never replaced, and clients never go back")
    print(f"{a.version} is after the published {published}")
    return 0


def cmd_verify(a) -> int:
    public = read_public(a.public_key)
    if a.url:
        body = fetch(f"{BASE}/{a.app}/latest.json")
        if body is None:
            die(f"{BASE}/{a.app}/latest.json does not exist")
        text = body.decode("utf-8")
    else:
        text = Path(a.file).read_text(encoding="utf-8")
    try:
        release = open_manifest(text, a.app, public)
    except Refused as e:
        die(f"refused: {e}")
    entries = release["files"] + release.get("downloads", [])
    if a.fetch:
        for e in entries:
            body = fetch(e["url"])
            if body is None:
                die(f"{e['url']} is named by the manifest but not there")
            if len(body) != e["size"] or hashlib.sha256(body).hexdigest() != e["sha256"]:
                die(f"{e['url']} is not the file the manifest describes")
    print(f"{a.app} {release['version']}: signed by the release key, "
          f"{len(entries)} file(s){', every one fetched and matching' if a.fetch else ''}")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="command", required=True)

    s = sub.add_parser("sign")
    s.add_argument("--app", required=True)
    s.add_argument("--version", required=True)
    s.add_argument("--dir", required=True)
    s.add_argument("--public-key", default="update.pub")
    s.set_defaults(run=cmd_sign)

    n = sub.add_parser("newer")
    n.add_argument("--app", required=True)
    n.add_argument("--version", required=True)
    n.set_defaults(run=cmd_newer)

    v = sub.add_parser("verify")
    v.add_argument("--app", required=True)
    v.add_argument("file", nargs="?")
    v.add_argument("--url", action="store_true", help=f"read {BASE}/<app>/latest.json")
    v.add_argument("--public-key", default="update.pub")
    v.add_argument("--fetch", action="store_true", help="download every file and check it")
    v.set_defaults(run=cmd_verify)

    a = p.parse_args()
    if a.command == "verify" and bool(a.file) == bool(a.url):
        p.error("verify takes a FILE or --url, one of the two")
    return a.run(a)


if __name__ == "__main__":
    raise SystemExit(main())
