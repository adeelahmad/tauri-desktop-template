import json
import pathlib
import subprocess
import sys
import unittest

WORKER = pathlib.Path(__file__).with_name("worker.py")


class WorkerTest(unittest.TestCase):
    def test_round_trip(self):
        reqs = [
            {"id": 1, "method": "ping"},
            {"id": 2, "method": "stats", "params": {"text": "The heart pumps blood. Blood flows!"}},
            {"id": 3, "method": "nope"},
        ]
        out = subprocess.run(
            [sys.executable, str(WORKER)],
            input="".join(json.dumps(r) + "\n" for r in reqs) + "not json\n",
            capture_output=True,
            text=True,
            check=True,
        ).stdout.splitlines()
        replies = [json.loads(line) for line in out]
        self.assertIn("python", replies[0]["result"])
        self.assertEqual(replies[1]["result"]["sentences"], 2)
        self.assertEqual(replies[1]["result"]["top"][0], "blood")
        self.assertIn("unknown method", replies[2]["error"])
        self.assertIn("bad JSON", replies[3]["error"])


if __name__ == "__main__":
    unittest.main()
