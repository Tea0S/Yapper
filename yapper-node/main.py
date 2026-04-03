#!/usr/bin/env python3
"""
Yapper Node: WebSocket bridge to the same sidecar protocol (JSON per message).
Run on your GPU machine:  python main.py --host 0.0.0.0 --port 8765 --token secret
Use Tailscale/WireGuard; do not expose raw to the public internet.
"""
from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
from pathlib import Path

import websockets


def find_server_py() -> Path:
    env = os.environ.get("YAPPER_SIDECAR")
    if env:
        return Path(env)
    root = Path(__file__).resolve().parent.parent
    return root / "sidecar" / "server.py"


async def bridge(ws, token: str) -> None:
    raw = await ws.recv()
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8")
    try:
        msg = json.loads(raw)
    except json.JSONDecodeError:
        await ws.close(code=4000, reason="invalid json")
        return
    if msg.get("type") != "auth" or msg.get("token") != token:
        await ws.close(code=4001, reason="unauthorized")
        return

    script = find_server_py()
    if not script.is_file():
        await ws.send(json.dumps({"type": "error", "message": f"sidecar missing: {script}"}))
        await ws.close()
        return

    python = os.environ.get("YAPPER_PYTHON", sys.executable)
    proc = await asyncio.create_subprocess_exec(
        python,
        str(script),
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL,
    )

    async def pump_out() -> None:
        assert proc.stdout
        while True:
            line = await proc.stdout.readline()
            if not line:
                break
            try:
                line = line.decode("utf-8").strip()
                if line:
                    await ws.send(line)
            except Exception:
                break

    pump = asyncio.create_task(pump_out())

    try:
        while True:
            incoming = await ws.recv()
            if isinstance(incoming, bytes):
                incoming = incoming.decode("utf-8")
            if not proc.stdin:
                break
            proc.stdin.write((incoming.strip() + "\n").encode("utf-8"))
            await proc.stdin.drain()
    except websockets.exceptions.ConnectionClosed:
        pass
    finally:
        pump.cancel()
        proc.terminate()
        try:
            await asyncio.wait_for(proc.wait(), timeout=2.0)
        except asyncio.TimeoutError:
            proc.kill()


async def main_async(host: str, port: int, token: str) -> None:
    async def handler(ws) -> None:
        await bridge(ws, token)

    async with websockets.serve(handler, host, port):
        print(f"Yapper Node listening on ws://{host}:{port}", flush=True)
        await asyncio.Future()


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--token", default=os.environ.get("YAPPER_TOKEN", "change-me"))
    args = ap.parse_args()
    asyncio.run(main_async(args.host, args.port, args.token))


if __name__ == "__main__":
    main()
