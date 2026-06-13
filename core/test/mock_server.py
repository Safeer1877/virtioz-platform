#!/usr/bin/env python3
"""
Virtioz Core Daemon — Mock WebSocket Server

A minimal echo server on ws://localhost:9001 that receives a JSON task,
adds "status": "completed", and sends it back.

Install dependency:
    pip install websockets

Usage:
    python mock_server.py
"""

import asyncio
import json
import websockets

HOST = "localhost"
PORT = 9001


async def handler(websocket):
    """Handle a single WebSocket connection."""
    print(f"[mock_server] Client connected: {websocket.remote_address}")

    async for message in websocket:
        print(f"[mock_server] Received: {message}")

        try:
            task = json.loads(message)
        except json.JSONDecodeError:
            task = {"raw": message}

        # Echo back with a "completed" status.
        task["status"] = "completed"
        task["result"] = "mock-inference-result-42"
        response = json.dumps(task)

        print(f"[mock_server] Sending:  {response}")
        await websocket.send(response)

    print(f"[mock_server] Client disconnected: {websocket.remote_address}")


async def main():
    print(f"[mock_server] Listening on ws://{HOST}:{PORT}")
    async with websockets.serve(handler, HOST, PORT):
        await asyncio.Future()  # run forever


if __name__ == "__main__":
    asyncio.run(main())
