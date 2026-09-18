"""Exercise real pipes with the Windows embeddable interpreter, without loading a model."""
import json
from pathlib import Path
import subprocess
import sys
import unittest


class StdioTests(unittest.TestCase):
    def test_legacy_windows_streams_are_normalized_before_ipc(self):
        sidecar_dir = str(Path(__file__).resolve().parent)
        child = f"""
import json, sys
sys.dont_write_bytecode = True
for stream in (sys.stdin, sys.stdout, sys.stderr):
    stream.reconfigure(encoding='cp1252')
sys.path.insert(0, {sidecar_dir!r})
import server
request = json.loads(sys.stdin.readline())
sys.stderr.write('diagnostic — café\\n')
server.emit({{'type': 'final', 'seq': 1, 'text': 'café — déjà vu',
             'received': request['text'],
             'encodings': [sys.stdin.encoding, sys.stdout.encoding, sys.stderr.encoding]}})
"""
        result = subprocess.run(
            [sys.executable, "-X", "utf8=0", "-c", child],
            input=json.dumps({"text": "François 中文"}, ensure_ascii=False).encode("utf-8"),
            capture_output=True, timeout=30,
        )
        self.assertEqual(result.returncode, 0, repr(result.stderr))
        message = json.loads(result.stdout.decode("utf-8"))
        self.assertEqual(message["text"], "café — déjà vu")
        self.assertEqual(message["received"], "François 中文")
        self.assertEqual(message["encodings"], ["utf-8"] * 3)
        self.assertIn("diagnostic — café", result.stderr.decode("utf-8"))


if __name__ == "__main__":
    unittest.main()
