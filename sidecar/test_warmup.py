"""Warm-up must finish before ready, never publish synthetic transcripts, and stay optional."""
import unittest
import sys
from pathlib import Path
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import server


class WarmupTests(unittest.TestCase):
    def test_eager_init_warms_before_ready(self):
        events = []
        with patch.object(server, "load_whisper_for_config"), \
             patch.object(server, "list_engines", return_value=["whisper"]), \
             patch.object(server, "warm_model", side_effect=lambda: events.append("warm")), \
             patch.object(server, "emit", side_effect=lambda e: events.append(e["type"])):
            server.handle_init({"engine": "whisper", "lazy_load": False})
        self.assertLess(events.index("warm"), events.index("ready"))
        self.assertNotIn("final", events)

    def test_lazy_init_does_not_warm(self):
        with patch.object(server, "unload_whisper"), \
             patch.object(server, "list_engines", return_value=["whisper"]), \
             patch.object(server, "warm_model") as warm, patch.object(server, "emit"):
            server.handle_init({"engine": "whisper", "lazy_load": True})
        warm.assert_not_called()

    def test_whisper_generator_is_consumed_without_emitting_text(self):
        consumed = []

        def segments():
            consumed.append(True)
            yield "synthetic output must never reach the user"

        model = Mock()
        model.transcribe.return_value = (segments(), None)
        with patch.multiple(server, MOCK=False, USE_MLX=False, MODEL=model, CONFIG={"engine": "whisper"}), \
             patch.object(server, "emit") as emit:
            server.warm_model()
        self.assertEqual(consumed, [True])
        emit.assert_not_called()

    def test_warmup_failure_is_nonfatal(self):
        model = Mock()
        model.transcribe.side_effect = RuntimeError("unsupported warm-up")
        with patch.multiple(server, MOCK=False, USE_MLX=False, MODEL=model, CONFIG={"engine": "whisper"}), \
             patch.object(server, "emit") as emit:
            server.warm_model()
        emit.assert_not_called()


if __name__ == "__main__":
    unittest.main()
