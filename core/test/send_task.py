#!/usr/bin/env python3
"""
Virtioz Core Daemon — Test Client

Writes a dummy task payload to the daemon's input path and waits for
the response at the output path.

Usage:
    python send_task.py
"""

import os
import sys
import time

# ---------------------------------------------------------------------------
# Paths (must match the daemon's constants)
# ---------------------------------------------------------------------------
if sys.platform == "win32":
    INPUT_PATH  = r"C:\tmp\virtioz_in"
    OUTPUT_PATH = r"C:\tmp\virtioz_out"
else:
    INPUT_PATH  = "/tmp/virtioz_in"
    OUTPUT_PATH = "/tmp/virtioz_out"

TIMEOUT_SECONDS = 15
POLL_INTERVAL   = 0.25        # seconds

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def main():
    # Ensure parent directory exists.
    os.makedirs(os.path.dirname(INPUT_PATH), exist_ok=True)

    # Clean up any stale output file.
    if os.path.exists(OUTPUT_PATH):
        os.remove(OUTPUT_PATH)
        print(f"[send_task] Removed stale output file: {OUTPUT_PATH}")

    # Write dummy payload.
    payload = "2 + 2"
    print(f"[send_task] Writing task to {INPUT_PATH}...")
    with open(INPUT_PATH, "w", encoding="utf-8") as f:
        f.write(payload)

    # Wait for the daemon to produce a response.
    print(f"[send_task] Waiting for response at {OUTPUT_PATH}...")
    elapsed = 0.0
    while elapsed < TIMEOUT_SECONDS:
        if os.path.exists(OUTPUT_PATH):
            # Small delay to ensure the daemon has finished writing.
            time.sleep(0.1)
            with open(OUTPUT_PATH, "r", encoding="utf-8") as f:
                response = f.read()
            # Clean up.
            os.remove(OUTPUT_PATH)
            print(f"[send_task] Response received:")
            print(response)
            return
        time.sleep(POLL_INTERVAL)
        elapsed += POLL_INTERVAL

    print(f"[send_task] ERROR: Timed out after {TIMEOUT_SECONDS}s — no response from daemon.", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
