#!/usr/bin/env python3
"""
SubReader Backend Integration Test Script
==========================================

Tests all API endpoints defined in openapi.yaml against a running backend.

Usage:
    1. Start the backend:  cargo run -p reader-backend
    2. Run tests:          python3 tests/test_backend.py [--base-url http://localhost:8080]

No external dependencies required — uses only Python standard library.
"""

import argparse
import hashlib
import json
import os
import ssl
import sys
import time
import uuid
from typing import Any, Optional
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urlparse
from urllib.request import Request, urlopen


# ─── HTTP Client (stdlib only) ───────────────────────────────────────────────

class Response:
    """Minimal response wrapper similar to requests.Response."""

    def __init__(self, status_code: int, body: bytes, headers: dict):
        self.status_code = status_code
        self.content = body
        self.text = body.decode("utf-8", errors="replace")
        self.headers = headers

    def json(self) -> Any:
        return json.loads(self.content)


def http_request(
    method: str,
    url: str,
    *,
    json_body: Optional[dict] = None,
    data: Optional[bytes] = None,
    headers: Optional[dict] = None,
    params: Optional[dict] = None,
    timeout: int = 10,
) -> Response:
    """Perform an HTTP request using only stdlib."""
    if params:
        url = f"{url}?{urlencode(params)}"

    req_headers = headers.copy() if headers else {}

    body = None
    if json_body is not None:
        body = json.dumps(json_body).encode("utf-8")
        req_headers.setdefault("Content-Type", "application/json")
    elif data is not None:
        body = data

    req = Request(url, data=body, headers=req_headers, method=method)

    # Allow self-signed certs in dev
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE

    try:
        with urlopen(req, timeout=timeout, context=ctx) as resp:
            return Response(
                status_code=resp.status,
                body=resp.read(),
                headers=dict(resp.headers),
            )
    except HTTPError as e:
        return Response(
            status_code=e.code,
            body=e.read(),
            headers=dict(e.headers) if e.headers else {},
        )


# ─── Helpers ─────────────────────────────────────────────────────────────────

class Colors:
    GREEN = "\033[92m"
    RED = "\033[91m"
    YELLOW = "\033[93m"
    CYAN = "\033[96m"
    BOLD = "\033[1m"
    RESET = "\033[0m"


class TestStats:
    def __init__(self):
        self.passed = 0
        self.failed = 0
        self.skipped = 0
        self.errors: list[str] = []

    def record_pass(self, name: str):
        self.passed += 1
        print(f"  {Colors.GREEN}✓ PASS{Colors.RESET}  {name}")

    def record_fail(self, name: str, reason: str):
        self.failed += 1
        self.errors.append(f"{name}: {reason}")
        print(f"  {Colors.RED}✗ FAIL{Colors.RESET}  {name}")
        print(f"         {Colors.RED}{reason}{Colors.RESET}")

    def record_skip(self, name: str, reason: str):
        self.skipped += 1
        print(f"  {Colors.YELLOW}⊘ SKIP{Colors.RESET}  {name} — {reason}")

    def summary(self):
        total = self.passed + self.failed + self.skipped
        print(f"\n{'─' * 60}")
        print(f"{Colors.BOLD}Results: {total} tests{Colors.RESET}")
        print(f"  {Colors.GREEN}Passed:  {self.passed}{Colors.RESET}")
        if self.failed:
            print(f"  {Colors.RED}Failed:  {self.failed}{Colors.RESET}")
        if self.skipped:
            print(f"  {Colors.YELLOW}Skipped: {self.skipped}{Colors.RESET}")
        if self.errors:
            print(f"\n{Colors.RED}Failures:{Colors.RESET}")
            for e in self.errors:
                print(f"  • {e}")
        print()
        return self.failed == 0


stats = TestStats()


def section(title: str):
    print(f"\n{Colors.CYAN}{Colors.BOLD}{'═' * 60}")
    print(f"  {title}")
    print(f"{'═' * 60}{Colors.RESET}")


def assert_status(resp: Response, expected: int, test_name: str) -> bool:
    if resp.status_code != expected:
        body = resp.text[:300]
        stats.record_fail(test_name, f"Expected {expected}, got {resp.status_code}: {body}")
        return False
    return True


def assert_json_key(data: dict, key: str, test_name: str) -> bool:
    if key not in data:
        stats.record_fail(test_name, f"Missing key '{key}' in response: {json.dumps(data)[:200]}")
        return False
    return True


# ─── Test Context ────────────────────────────────────────────────────────────

class Ctx:
    """Shared test context to pass tokens/IDs between tests."""

    def __init__(self, base_url: str):
        self.base = base_url.rstrip("/")
        # Will be populated during tests
        self.user_id: Optional[str] = None
        self.access_token: Optional[str] = None
        self.refresh_token: Optional[str] = None
        self.device_id: str = f"test-device-{uuid.uuid4().hex[:8]}"
        self.device_id_2: str = f"test-device2-{uuid.uuid4().hex[:8]}"
        self.username: str = f"testuser_{uuid.uuid4().hex[:6]}"
        self.email: str = f"test_{uuid.uuid4().hex[:6]}@example.com"
        self.password: str = "TestP@ssw0rd123"
        self.new_password: str = "NewP@ssw0rd456"
        # File test state
        self.upload_id: Optional[str] = None
        self.file_id: Optional[str] = None

    def auth_headers(self, token: Optional[str] = None) -> dict:
        t = token or self.access_token
        return {"Authorization": f"Bearer {t}"} if t else {}

    def url(self, path: str) -> str:
        return f"{self.base}{path}"

    # ── Convenience HTTP methods ──

    def get(self, path: str, *, headers: Optional[dict] = None, params: Optional[dict] = None) -> Response:
        return http_request("GET", self.url(path), headers=headers, params=params)

    def post(self, path: str, *, json_body: Optional[dict] = None, headers: Optional[dict] = None) -> Response:
        return http_request("POST", self.url(path), json_body=json_body, headers=headers)

    def put(self, path: str, *, json_body: Optional[dict] = None, data: Optional[bytes] = None, headers: Optional[dict] = None) -> Response:
        return http_request("PUT", self.url(path), json_body=json_body, data=data, headers=headers)

    def delete(self, path: str, *, headers: Optional[dict] = None) -> Response:
        return http_request("DELETE", self.url(path), headers=headers)


# ─── Health Tests ────────────────────────────────────────────────────────────

def test_health(ctx: Ctx):
    section("Health Check")

    name = "GET /health returns 200 with status and version"
    try:
        resp = ctx.get("/health")
        if not assert_status(resp, 200, name):
            return
        data = resp.json()
        if assert_json_key(data, "status", name) and assert_json_key(data, "version", name):
            stats.record_pass(name)
    except (URLError, ConnectionError, OSError) as e:
        stats.record_fail(name, f"Cannot connect to {ctx.base}: {e}")
        print(f"\n{Colors.RED}Backend not reachable — aborting.{Colors.RESET}")
        sys.exit(1)


# ─── Auth Tests ──────────────────────────────────────────────────────────────

def test_register(ctx: Ctx):
    section("Auth — Register")

    # 1. Successful registration
    name = "POST /auth/register — success"
    resp = ctx.post("/auth/register", json_body={
        "username": ctx.username,
        "email": ctx.email,
        "password": ctx.password,
    })
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "user_id", name) and assert_json_key(data, "message", name):
            ctx.user_id = data["user_id"]
            stats.record_pass(name)

    # 2. Duplicate username
    name = "POST /auth/register — duplicate username → 409"
    resp = ctx.post("/auth/register", json_body={
        "username": ctx.username,
        "email": f"other_{uuid.uuid4().hex[:6]}@example.com",
        "password": ctx.password,
    })
    if assert_status(resp, 409, name):
        stats.record_pass(name)

    # 3. Short password
    name = "POST /auth/register — short password → 400"
    resp = ctx.post("/auth/register", json_body={
        "username": f"u_{uuid.uuid4().hex[:6]}",
        "email": f"e_{uuid.uuid4().hex[:6]}@example.com",
        "password": "short",
    })
    if assert_status(resp, 400, name):
        stats.record_pass(name)

    # 4. Invalid email
    name = "POST /auth/register — invalid email → 400"
    resp = ctx.post("/auth/register", json_body={
        "username": f"u_{uuid.uuid4().hex[:6]}",
        "email": "not-an-email",
        "password": ctx.password,
    })
    if assert_status(resp, 400, name):
        stats.record_pass(name)

    # 5. Short username
    name = "POST /auth/register — short username → 400"
    resp = ctx.post("/auth/register", json_body={
        "username": "ab",
        "email": f"e_{uuid.uuid4().hex[:6]}@example.com",
        "password": ctx.password,
    })
    if assert_status(resp, 400, name):
        stats.record_pass(name)


def test_login(ctx: Ctx):
    section("Auth — Login")

    # 1. Login with username
    name = "POST /auth/login — success with username"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.password,
        "device_id": ctx.device_id,
        "device_name": "Test Laptop",
        "platform": "test",
    })
    if assert_status(resp, 200, name):
        data = resp.json()
        ok = all([
            assert_json_key(data, "access_token", name),
            assert_json_key(data, "refresh_token", name),
            assert_json_key(data, "expires_in", name),
            assert_json_key(data, "user_id", name),
        ])
        if ok:
            ctx.access_token = data["access_token"]
            ctx.refresh_token = data["refresh_token"]
            stats.record_pass(name)

    # 2. Login with email
    name = "POST /auth/login — success with email"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.email,
        "password": ctx.password,
        "device_id": ctx.device_id_2,
        "device_name": "Test Phone",
        "platform": "test",
    })
    if assert_status(resp, 200, name):
        stats.record_pass(name)

    # 3. Wrong password
    name = "POST /auth/login — wrong password → 401"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": "WrongPassword123",
        "device_id": ctx.device_id,
    })
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 4. Non-existent user
    name = "POST /auth/login — non-existent user → 401"
    resp = ctx.post("/auth/login", json_body={
        "credential": "no_such_user_xyz",
        "password": ctx.password,
        "device_id": ctx.device_id,
    })
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 5. Same device re-login replaces old refresh token
    name = "POST /auth/login — same device re-login returns new tokens"
    old_refresh = ctx.refresh_token
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.password,
        "device_id": ctx.device_id,
        "device_name": "Test Laptop",
        "platform": "test",
    })
    if assert_status(resp, 200, name):
        data = resp.json()
        ctx.access_token = data["access_token"]
        ctx.refresh_token = data["refresh_token"]
        if ctx.refresh_token != old_refresh:
            stats.record_pass(name)
        else:
            stats.record_fail(name, "Re-login should return a different refresh token")


def test_refresh(ctx: Ctx):
    section("Auth — Refresh Token")

    if not ctx.refresh_token:
        stats.record_skip("Refresh tests", "No refresh token available")
        return

    # 1. Successful refresh
    name = "POST /auth/refresh — success"
    old_refresh = ctx.refresh_token
    resp = ctx.post("/auth/refresh", json_body={
        "refresh_token": ctx.refresh_token,
        "device_id": ctx.device_id,
    })
    if assert_status(resp, 200, name):
        data = resp.json()
        ok = all([
            assert_json_key(data, "access_token", name),
            assert_json_key(data, "refresh_token", name),
            assert_json_key(data, "expires_in", name),
            assert_json_key(data, "user_id", name),
        ])
        if ok:
            ctx.access_token = data["access_token"]
            ctx.refresh_token = data["refresh_token"]
            stats.record_pass(name)

    # 2. Old refresh token should be rejected (rotation)
    name = "POST /auth/refresh — reuse old token → 401"
    resp = ctx.post("/auth/refresh", json_body={
        "refresh_token": old_refresh,
        "device_id": ctx.device_id,
    })
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 3. After reuse detection, re-login to get fresh tokens
    name = "POST /auth/login — re-login after reuse detection"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.password,
        "device_id": ctx.device_id,
        "device_name": "Test Laptop",
        "platform": "test",
    })
    if assert_status(resp, 200, name):
        data = resp.json()
        ctx.access_token = data["access_token"]
        ctx.refresh_token = data["refresh_token"]
        stats.record_pass(name)

    # 4. Wrong device_id
    name = "POST /auth/refresh — wrong device_id → 401"
    resp = ctx.post("/auth/refresh", json_body={
        "refresh_token": ctx.refresh_token,
        "device_id": "wrong-device-id",
    })
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 5. Re-login again to recover valid tokens
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.password,
        "device_id": ctx.device_id,
        "device_name": "Test Laptop",
        "platform": "test",
    })
    if resp.status_code == 200:
        data = resp.json()
        ctx.access_token = data["access_token"]
        ctx.refresh_token = data["refresh_token"]

    # 6. Bogus refresh token
    name = "POST /auth/refresh — bogus token → 401"
    resp = ctx.post("/auth/refresh", json_body={
        "refresh_token": "totally-fake-token",
        "device_id": ctx.device_id,
    })
    if assert_status(resp, 401, name):
        stats.record_pass(name)


# ─── Device Tests ────────────────────────────────────────────────────────────

def test_devices(ctx: Ctx):
    section("Devices")

    if not ctx.access_token:
        stats.record_skip("Device tests", "No access token available")
        return

    # 1. List devices
    name = "GET /auth/devices — list active devices"
    resp = ctx.get("/auth/devices", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "devices", name):
            devices = data["devices"]
            if len(devices) >= 1:
                stats.record_pass(name)
            else:
                stats.record_fail(name, f"Expected at least 1 device, got {len(devices)}")

    # 2. List devices without auth
    name = "GET /auth/devices — no auth → 401"
    resp = ctx.get("/auth/devices")
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 3. Login a third device, then remove it
    third_device = f"test-device3-{uuid.uuid4().hex[:8]}"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.password,
        "device_id": third_device,
        "device_name": "Removable Device",
        "platform": "test",
    })
    if resp.status_code != 200:
        stats.record_skip("DELETE /auth/devices/:id", "Could not login third device")
        return

    name = f"DELETE /auth/devices/{third_device} — remove device"
    resp = ctx.delete(f"/auth/devices/{third_device}", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "message", name):
            stats.record_pass(name)

    # 4. Verify removed device no longer in list
    name = "GET /auth/devices — removed device not listed"
    resp = ctx.get("/auth/devices", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        device_ids = [d["device_id"] for d in data["devices"]]
        if third_device not in device_ids:
            stats.record_pass(name)
        else:
            stats.record_fail(name, f"Removed device '{third_device}' still in list")


# ─── Sync Tests ──────────────────────────────────────────────────────────────

def test_sync(ctx: Ctx):
    section("Sync — Push & Pull")

    if not ctx.access_token:
        stats.record_skip("Sync tests", "No access token available")
        return

    # 1. Push operations
    name = "POST /sync/push — push 3 operations"
    ops = [
        {
            "op_id": f"op-{uuid.uuid4().hex[:8]}",
            "op_type": "UpdateProgress",
            "op_data": json.dumps({"book_id": "book-1", "progress": 0.25}),
            "hlc_ts": int(time.time() * 1000),
        },
        {
            "op_id": f"op-{uuid.uuid4().hex[:8]}",
            "op_type": "AddBookmark",
            "op_data": json.dumps({"book_id": "book-1", "page": 42}),
            "hlc_ts": int(time.time() * 1000) + 1,
        },
        {
            "op_id": f"op-{uuid.uuid4().hex[:8]}",
            "op_type": "UpdateProgress",
            "op_data": json.dumps({"book_id": "book-1", "progress": 0.50}),
            "hlc_ts": int(time.time() * 1000) + 2,
        },
    ]
    resp = ctx.post("/sync/push", json_body={"operations": ops}, headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "accepted_count", name):
            if data["accepted_count"] == 3:
                stats.record_pass(name)
            else:
                stats.record_fail(name, f"Expected accepted_count=3, got {data['accepted_count']}")

    # 2. Push duplicate (idempotent)
    name = "POST /sync/push — duplicate ops are skipped"
    resp = ctx.post("/sync/push", json_body={"operations": ops[:1]}, headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if data.get("accepted_count") == 0:
            stats.record_pass(name)
        else:
            stats.record_fail(name, f"Expected accepted_count=0 for duplicate, got {data.get('accepted_count')}")

    # 3. Push empty batch → 400
    name = "POST /sync/push — empty batch → 400"
    resp = ctx.post("/sync/push", json_body={"operations": []}, headers=ctx.auth_headers())
    if assert_status(resp, 400, name):
        stats.record_pass(name)

    # 4. Push without auth → 401
    name = "POST /sync/push — no auth → 401"
    resp = ctx.post("/sync/push", json_body={"operations": ops[:1]})
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 5. Pull operations
    name = "GET /sync/pull — pull from cursor=0"
    resp = ctx.get("/sync/pull", params={"cursor": 0, "limit": 100}, headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        ok = all([
            assert_json_key(data, "operations", name),
            assert_json_key(data, "next_cursor", name),
            assert_json_key(data, "has_more", name),
        ])
        if ok:
            if len(data["operations"]) >= 3:
                stats.record_pass(name)
            else:
                stats.record_fail(name, f"Expected ≥3 operations, got {len(data['operations'])}")

    # 6. Pull with cursor beyond end
    name = "GET /sync/pull — cursor beyond end returns empty"
    resp = ctx.get("/sync/pull", params={"cursor": 999999999, "limit": 10}, headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if len(data.get("operations", [])) == 0 and data.get("has_more") is False:
            stats.record_pass(name)
        else:
            stats.record_fail(name, f"Expected empty result, got {len(data.get('operations', []))} ops")


# ─── File Tests ──────────────────────────────────────────────────────────────

def test_files(ctx: Ctx):
    section("Files — Upload, Download, List, Delete")

    if not ctx.access_token:
        stats.record_skip("File tests", "No access token available")
        return

    # Create test file content
    file_content = os.urandom(1024 * 10)  # 10 KB random data
    file_sha256 = hashlib.sha256(file_content).hexdigest()
    file_name = f"test-file-{uuid.uuid4().hex[:8]}.bin"
    file_size = len(file_content)
    chunk_size = 4096  # 4 KB chunks

    # 1. Init upload
    name = "POST /files/upload/init — initialize upload"
    resp = ctx.post("/files/upload/init", json_body={
        "file_name": file_name,
        "file_size": file_size,
        "sha256": file_sha256,
        "chunk_size": chunk_size,
    }, headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        ok = all([
            assert_json_key(data, "upload_id", name),
            assert_json_key(data, "chunk_size", name),
            assert_json_key(data, "total_chunks", name),
        ])
        if ok:
            ctx.upload_id = data["upload_id"]
            actual_chunk_size = data["chunk_size"]
            total_chunks = data["total_chunks"]
            stats.record_pass(name)
    else:
        stats.record_skip("File upload tests", "Init failed")
        return

    # 2. Upload chunks
    all_chunks_ok = True
    for i in range(total_chunks):
        start = i * actual_chunk_size
        end = min(start + actual_chunk_size, file_size)
        chunk_data = file_content[start:end]

        chunk_name = f"PUT /files/upload/{ctx.upload_id}/chunk/{i}"
        resp = ctx.put(
            f"/files/upload/{ctx.upload_id}/chunk/{i}",
            data=chunk_data,
            headers={
                **ctx.auth_headers(),
                "Content-Type": "application/octet-stream",
            },
        )
        if not assert_status(resp, 200, chunk_name):
            all_chunks_ok = False
            break

    name = f"Upload all {total_chunks} chunks"
    if all_chunks_ok:
        stats.record_pass(name)
    else:
        stats.record_skip("File complete/download", "Chunk upload failed")
        return

    # 3. Complete upload
    name = "POST /files/upload/{upload_id}/complete — finalize"
    resp = ctx.post(f"/files/upload/{ctx.upload_id}/complete", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        ok = all([
            assert_json_key(data, "file_id", name),
            assert_json_key(data, "file_name", name),
            assert_json_key(data, "sha256", name),
        ])
        if ok:
            ctx.file_id = data["file_id"]
            if data["sha256"] == file_sha256:
                stats.record_pass(name)
            else:
                stats.record_fail(name, f"SHA-256 mismatch: expected {file_sha256}, got {data['sha256']}")
    else:
        stats.record_skip("File download/list/delete", "Complete failed")
        return

    # 4. Duplicate upload → 409
    name = "POST /files/upload/init — duplicate SHA-256 → 409"
    resp = ctx.post("/files/upload/init", json_body={
        "file_name": file_name,
        "file_size": file_size,
        "sha256": file_sha256,
    }, headers=ctx.auth_headers())
    if assert_status(resp, 409, name):
        stats.record_pass(name)

    # 5. List files
    name = "GET /files — list files contains uploaded file"
    resp = ctx.get("/files", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "files", name):
            file_ids = [f["file_id"] for f in data["files"]]
            if ctx.file_id in file_ids:
                stats.record_pass(name)
            else:
                stats.record_fail(name, f"Uploaded file {ctx.file_id} not in list")

    # 6. Download file
    name = f"GET /files/{ctx.file_id} — download and verify content"
    resp = ctx.get(f"/files/{ctx.file_id}", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        downloaded = resp.content
        if downloaded == file_content:
            stats.record_pass(name)
        else:
            dl_hash = hashlib.sha256(downloaded).hexdigest()
            stats.record_fail(
                name,
                f"Content mismatch: expected {len(file_content)} bytes (sha256={file_sha256}), "
                f"got {len(downloaded)} bytes (sha256={dl_hash})",
            )

    # 7. Download non-existent file → 404
    name = "GET /files/{fake_id} — not found → 404"
    fake_id = str(uuid.uuid4())
    resp = ctx.get(f"/files/{fake_id}", headers=ctx.auth_headers())
    if assert_status(resp, 404, name):
        stats.record_pass(name)

    # 8. Delete file
    name = f"DELETE /files/{ctx.file_id} — delete file"
    resp = ctx.delete(f"/files/{ctx.file_id}", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "message", name):
            stats.record_pass(name)

    # 9. Download deleted file → 404
    name = "GET /files/{deleted_id} — deleted file → 404"
    resp = ctx.get(f"/files/{ctx.file_id}", headers=ctx.auth_headers())
    if assert_status(resp, 404, name):
        stats.record_pass(name)


# ─── Auth — Change Password ─────────────────────────────────────────────────

def test_change_password(ctx: Ctx):
    section("Auth — Change Password")

    if not ctx.access_token:
        stats.record_skip("Change password tests", "No access token available")
        return

    # 1. Wrong old password
    name = "PUT /auth/password — wrong old password → 401"
    resp = ctx.put(
        "/auth/password",
        json_body={"old_password": "WrongOldPassword", "new_password": ctx.new_password},
        headers=ctx.auth_headers(),
    )
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 2. New password too short
    name = "PUT /auth/password — new password too short → 400"
    resp = ctx.put(
        "/auth/password",
        json_body={"old_password": ctx.password, "new_password": "short"},
        headers=ctx.auth_headers(),
    )
    if assert_status(resp, 400, name):
        stats.record_pass(name)

    # 3. Successful password change
    name = "PUT /auth/password — success"
    resp = ctx.put(
        "/auth/password",
        json_body={"old_password": ctx.password, "new_password": ctx.new_password},
        headers=ctx.auth_headers(),
    )
    if assert_status(resp, 200, name):
        stats.record_pass(name)

    # 4. Re-login with new password
    name = "POST /auth/login — login with new password"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.new_password,
        "device_id": ctx.device_id,
        "device_name": "Test Laptop",
        "platform": "test",
    })
    if assert_status(resp, 200, name):
        data = resp.json()
        ctx.access_token = data["access_token"]
        ctx.refresh_token = data["refresh_token"]
        ctx.password = ctx.new_password
        stats.record_pass(name)

    # 5. Old password should no longer work
    name = "POST /auth/login — old password rejected → 401"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": "TestP@ssw0rd123",  # original password
        "device_id": ctx.device_id,
    })
    if assert_status(resp, 401, name):
        stats.record_pass(name)


# ─── Auth — Logout ───────────────────────────────────────────────────────────

def test_logout(ctx: Ctx):
    section("Auth — Logout")

    if not ctx.access_token:
        stats.record_skip("Logout tests", "No access token available")
        return

    old_token = ctx.access_token

    # 1. Successful logout
    name = "POST /auth/logout — success"
    resp = ctx.post("/auth/logout", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "message", name):
            stats.record_pass(name)

    # 2. Old access token should be rejected
    name = "GET /auth/devices — old token after logout → 401"
    resp = ctx.get("/auth/devices", headers=ctx.auth_headers(old_token))
    if assert_status(resp, 401, name):
        stats.record_pass(name)

    # 3. Re-login to continue tests
    name = "POST /auth/login — re-login after logout"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.password,
        "device_id": ctx.device_id,
        "device_name": "Test Laptop",
        "platform": "test",
    })
    if assert_status(resp, 200, name):
        data = resp.json()
        ctx.access_token = data["access_token"]
        ctx.refresh_token = data["refresh_token"]
        stats.record_pass(name)


# ─── Auth — Delete Account ──────────────────────────────────────────────────

def test_delete_account(ctx: Ctx):
    section("Auth — Delete Account")

    if not ctx.access_token:
        stats.record_skip("Delete account tests", "No access token available")
        return

    # 1. Delete account
    name = "DELETE /auth/account — success"
    resp = ctx.delete("/auth/account", headers=ctx.auth_headers())
    if assert_status(resp, 200, name):
        data = resp.json()
        if assert_json_key(data, "message", name):
            stats.record_pass(name)

    # 2. Login with deleted account should fail
    name = "POST /auth/login — deleted account → 401"
    resp = ctx.post("/auth/login", json_body={
        "credential": ctx.username,
        "password": ctx.password,
        "device_id": ctx.device_id,
    })
    if assert_status(resp, 401, name):
        stats.record_pass(name)


# ─── Main ────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="SubReader Backend Integration Tests")
    parser.add_argument(
        "--base-url",
        default="http://localhost:8080",
        help="Backend base URL (default: http://localhost:8080)",
    )
    args = parser.parse_args()

    print(f"\n{Colors.BOLD}SubReader Backend Integration Tests{Colors.RESET}")
    print(f"Target: {args.base_url}\n")

    ctx = Ctx(args.base_url)

    # Run tests in dependency order
    test_health(ctx)
    test_register(ctx)
    test_login(ctx)
    test_refresh(ctx)
    test_devices(ctx)
    test_sync(ctx)
    test_files(ctx)
    test_change_password(ctx)
    test_logout(ctx)
    test_delete_account(ctx)  # Must be last — destroys the test user

    ok = stats.summary()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
