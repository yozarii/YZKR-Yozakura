"""Hermetic tests for the escalation adapter: a stub `claude` script, never the real CLI."""
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import escalate  # noqa: E402

ESCALATE = escalate.__file__


class StubClaude:
    """A fake `claude` that records how it was called and prints a scripted reply."""

    def __init__(self, reply, code=0):
        self.dir = tempfile.TemporaryDirectory()
        self.log = os.path.join(self.dir.name, "call.json")
        self.path = os.path.join(self.dir.name, "claude")
        with open(self.path, "w") as f:
            f.write(
                "#!/usr/bin/env python3\nimport json, os, sys\n"
                f"json.dump({{'argv': sys.argv[1:], 'cwd': os.getcwd()}}, open({self.log!r}, 'w'))\n"
                f"print({reply!r}); sys.exit({code})\n"
            )
        os.chmod(self.path, os.stat(self.path).st_mode | stat.S_IXUSR)

    def call(self):
        with open(self.log) as f:
            return json.load(f)

    def close(self):
        self.dir.cleanup()


REQ = {"task": "pick the official vendor site for this app", "query": "ghost",
       "candidates": [{"id": "a.example", "text": "a.example: Ghost blog"}, {"id": "ghost.example", "text": "ghost.example: Ghost"}]}


def run(req, stub, env=None):
    e = {**os.environ, "KAIKA_CLAUDE_BIN": stub.path, **(env or {})}
    p = subprocess.run([sys.executable, "-I", ESCALATE], input=json.dumps(req), capture_output=True, text=True, env=e)
    assert p.returncode == 0, p.stderr
    return json.loads(p.stdout)


class EscalateTests(unittest.TestCase):
    def test_the_letter_maps_back_to_the_candidate_id(self):
        s = StubClaude("B")
        self.addCleanup(s.close)
        out = run(REQ, s)
        self.assertEqual((out["choice"], out["confidence"]), ("ghost.example", escalate.DECISIVE))

    def test_it_is_locked_down(self):
        s = StubClaude("A")
        self.addCleanup(s.close)
        run(REQ, s)
        call = s.call()
        argv = call["argv"]
        self.assertEqual(argv[argv.index("--tools") + 1], "", "all tools must be disabled")
        for flag in ("--no-session-persistence", "--disable-slash-commands", "--strict-mcp-config"):
            self.assertIn(flag, argv)
        self.assertEqual(argv[argv.index("--setting-sources") + 1], "project", "user hooks must not load")
        self.assertEqual(argv[argv.index("--model") + 1], "haiku")
        self.assertNotIn("YZKR", call["cwd"], "must not run inside the user's project")
        self.assertTrue(os.path.realpath(call["cwd"]).startswith(os.path.realpath(tempfile.gettempdir())) or "tmp" in call["cwd"].lower())

    def test_only_public_candidate_data_is_in_the_prompt(self):
        s = StubClaude("A")
        self.addCleanup(s.close)
        run(REQ, s)
        argv = s.call()["argv"]
        prompt = argv[argv.index("-p") + 1]
        self.assertIn("A) a.example: Ghost blog", prompt)
        self.assertIn("X) none of these", prompt)
        self.assertNotIn(os.path.expanduser("~"), " ".join(argv))

    def test_the_model_can_be_chosen(self):
        s = StubClaude("A")
        self.addCleanup(s.close)
        run(REQ, s, env={"KAIKA_ESCALATION_MODEL": "sonnet"})
        argv = s.call()["argv"]
        self.assertEqual(argv[argv.index("--model") + 1], "sonnet")

    def test_none_noise_and_failure_are_all_unsure(self):
        for reply, code in (("X", 0), ("I think it is the first one because", 0), ("", 0), ("A", 1)):
            s = StubClaude(reply, code)
            self.addCleanup(s.close)
            out = run(REQ, s)
            self.assertIsNone(out["choice"], reply)
            self.assertEqual(out["confidence"], 0.0)

    def test_a_missing_binary_is_unsure_not_a_crash(self):
        p = subprocess.run([sys.executable, "-I", ESCALATE], input=json.dumps(REQ), capture_output=True, text=True,
                           env={**os.environ, "KAIKA_CLAUDE_BIN": "/definitely/not/claude"})
        self.assertEqual(p.returncode, 0)
        self.assertIsNone(json.loads(p.stdout)["choice"])

    def test_a_lone_candidate_needs_no_model(self):
        out = escalate.answer({"candidates": [{"id": "only", "text": "t"}]}, runner=lambda *a: (_ for _ in ()).throw(AssertionError("must not call")))
        self.assertEqual(out["choice"], "only")


if __name__ == "__main__":
    unittest.main()
