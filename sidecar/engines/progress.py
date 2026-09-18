"""Machine-readable startup progress on stderr; stdout remains transcription IPC."""
import json
import sys


def report(stage: str, message: str) -> None:
    sys.stderr.write("YAPPER_PROGRESS " + json.dumps({"stage": stage, "message": message}) + "\n")
    sys.stderr.flush()
