"""Bounded adapter component checks; real rotating Hermes acceptance is separate."""
import json
import pathlib
import tempfile
import threading
import time
import unittest
from test_hermes_plugin import plugin


class NativeReader:
    def __init__(self, rows, chain, child=None, blocked=None):
        for session, row in rows.items():
            row.setdefault("id", session)
            row.setdefault("parent_session_id", None)
            row.setdefault("ended_at", None)
            row.setdefault("end_reason", None)
        self.rows, self.chain, self.child, self.blocked = rows, chain, child, blocked
        self.get_calls = self.chain_calls = self.child_calls = 0
        self.read_only = True

    def get_session(self, session):
        self.get_calls += 1
        if self.blocked:
            self.blocked.wait(2)
        return self.rows.get(session)

    def get_compression_chain(self, session):
        self.chain_calls += 1
        return self.chain[self.chain.index(session):] if session in self.chain else [session]

    def find_live_compression_child(self, session):
        self.child_calls += 1
        return self.rows.get(self.child) if self.child else None

    def close(self):
        pass


class LineageTests(unittest.TestCase):
    def manager(self, reader):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = pathlib.Path(directory.name) / "state.db"
        path.touch()
        stat = path.stat()

        def factory(*, db_path, read_only):
            self.assertEqual(db_path, path)
            self.assertIs(read_only, True)
            return reader

        return plugin._LineageReader(path, (stat.st_dev, stat.st_ino), factory)

    def warm(self, manager, current):
        manager.header(current)
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            with manager.lock:
                if current in manager.cache:
                    return
            time.sleep(.001)
        self.fail("background reader did not finish")

    def test_an_idle_profile_worker_can_be_retired(self):
        manager = self.manager(NativeReader({"root": {}}, ["root"]))
        self.warm(manager, "root")
        deadline = time.monotonic() + 1
        while not manager.retire_idle() and time.monotonic() < deadline:
            time.sleep(.001)
        self.assertFalse(manager.worker.is_alive())

    def test_020_compatibility_never_enumerates_unbounded_lineage_and_survives_restart(self):
        class LegacyReader(NativeReader):
            get_compression_chain = None

            def get_compression_lineage(self, _session):
                raise AssertionError("unbounded legacy SDK method must never run")

        rows = {"a": {"ended_at": 1, "end_reason": "compression"},
                "b": {"parent_session_id": "a", "ended_at": 2, "end_reason": "compression"},
                "c": {"parent_session_id": "b", "model_config": "{}"}}
        for _ in range(2):
            reader = LegacyReader(rows, [], "c")
            manager = self.manager(reader)
            self.warm(manager, "c")
            with manager.lock:
                self.assertEqual(manager.cache["c"]["ids"], ["a", "b", "c"])
                self.assertTrue(manager.cache["c"]["complete"])
            self.assertEqual(reader.chain_calls, 0)
            self.assertLessEqual(reader.get_calls, 6)

    def test_020_ordinary_children_forks_and_ambiguous_compression_never_join_main(self):
        class LegacyReader(NativeReader):
            get_compression_chain = None
            get_compression_lineage = lambda *args: []

        for fields, live_child in (({"source": "tool"}, "b"),
                                   ({"model_config": '{"_branched_from":"a"}'}, "b"),
                                   ({"model_config": '{"_delegate_from":"a"}'}, "b"),
                                   ({"model_config": "{"}, "b"),
                                   ({"model_config": "x" * 16385}, "b"), ({}, None)):
            rows = {"a": {"ended_at": 1, "end_reason": "compression"},
                    "b": {"parent_session_id": "a", **fields}}
            manager = self.manager(LegacyReader(rows, [], live_child))
            self.warm(manager, "b")
            with manager.lock:
                self.assertEqual(manager.cache["b"]["ids"], ["b"])
                self.assertFalse(manager.cache["b"]["complete"])

    def test_missing_native_schema_does_not_invent_an_open_root(self):
        reader = NativeReader({"root": {}}, ["root"])
        reader.rows["root"].pop("parent_session_id")
        manager = self.manager(reader)
        self.assertIsNone(manager.header("root"))
        self.assertEqual(reader.chain_calls, 0)

    def test_creation_time_comes_from_the_fresh_native_row_not_the_proof_stamp(self):
        for started, expected in ((1234.5, 1234500), (None, None), (-1, None),
                                  (True, None), ("1234", None), (float("nan"), None)):
            manager = self.manager(NativeReader({"root": {"started_at": started}}, ["root"]))
            self.warm(manager, "root")
            header = json.loads(manager.header("root"))
            self.assertEqual(header["current_state"].get("started_at_ms"), expected)
            self.assertEqual(header["ids"], ["root"])

    def test_warm_ancestor_proof_survives_normal_generation_interval(self):
        rows = {"parent": {"parent_session_id": None, "ended_at": 1, "end_reason": "compression"},
                "child": {"parent_session_id": "parent", "ended_at": None, "id": "child"}}
        manager = self.manager(NativeReader(rows, ["parent", "child"], "child"))
        self.warm(manager, "child")
        with manager.lock:
            manager.cache["child"]["sample"]["_monotonic"] -= 5
            original = manager.cache["child"]["sample"]["sequence"]
        # Hold the worker in another native read: a hot request uses the old
        # ancestor proof but may not borrow a cached current/open observation.
        gate = threading.Event()
        manager.factory = lambda **_: NativeReader(rows, ["parent", "child"], "child", gate)
        manager.schedule("child")
        time.sleep(.01)
        started = time.monotonic()
        header = json.loads(manager.header("child"))
        elapsed = time.monotonic() - started
        gate.set()
        self.assertLess(elapsed, .15)
        self.assertEqual(header["ids"], ["parent", "child"])
        self.assertEqual(header["proof_observation"]["sequence"], original)
        self.assertGreaterEqual(header["proof_observation"]["age_ms"], 5000)
        self.assertNotIn("current_state", header)

    def test_180_depth_is_bounded_and_suffix_is_incomplete(self):
        ids = [f"s{i}" for i in range(180)]
        rows = {s: {"id": s, "parent_session_id": ids[i - 1] if i else None,
                    "ended_at": None if i == 179 else 1, "end_reason": "compression"}
                for i, s in enumerate(ids)}
        reader = NativeReader(rows, ids, ids[-1])
        manager = self.manager(reader)
        self.warm(manager, ids[-1])
        with manager.lock:
            proof = manager.cache[ids[-1]].copy()
        self.assertEqual(proof["ids"], ids[-32:])
        self.assertFalse(proof["complete"])
        # One header schedules an own-row lookup and its coalesced refresh.
        self.assertLessEqual(reader.get_calls, 64)
        self.assertLessEqual(reader.chain_calls, 2)

    def test_observed_withdrawal_survives_generation_without_refreshing_its_stamp(self):
        rows = {"parent": {"parent_session_id": None, "ended_at": None},
                "child": {"parent_session_id": "parent", "ended_at": None}}
        manager = self.manager(NativeReader(rows, ["parent"]))
        self.warm(manager, "child")
        with manager.lock:
            manager.cache["child"]["sample"]["_monotonic"] -= 15
            original = manager.cache["child"]["sample"]["sequence"]
        gate = threading.Event()
        manager.factory = lambda **_: NativeReader(rows, ["parent"], blocked=gate)
        manager.schedule("child")
        time.sleep(.01)
        try:
            header = json.loads(manager.header("child"))
        finally:
            gate.set()
        self.assertEqual(header["invalid_edges"]["edges"], [["parent", "child"]])
        self.assertEqual(header["invalid_edges"]["observation"]["sequence"], original)
        self.assertGreaterEqual(header["invalid_edges"]["observation"]["age_ms"], 15000)
        self.assertNotIn("current_state", header)

    def test_fork_does_not_withdraw_confirmed_ancestor_prefix(self):
        rows = {"a": {"id": "a", "parent_session_id": None, "ended_at": 1, "end_reason": "compression"},
                "b": {"id": "b", "parent_session_id": "a", "ended_at": 2, "end_reason": "compression"},
                "fork": {"id": "fork", "parent_session_id": "b", "ended_at": None}}
        manager = self.manager(NativeReader(rows, ["a", "b", "canonical"], "canonical"))
        self.warm(manager, "fork")
        with manager.lock:
            proof = manager.cache["fork"]
            self.assertEqual(proof["ids"], ["fork"])
            self.assertEqual(proof["invalid"], [["b", "fork"]])

    def test_ambiguous_live_child_is_a_gap(self):
        rows = {"a": {"id": "a", "parent_session_id": None, "ended_at": 1, "end_reason": "compression"},
                "b": {"id": "b", "parent_session_id": "a", "ended_at": None}}
        manager = self.manager(NativeReader(rows, ["a", "b"], None))
        self.warm(manager, "b")
        with manager.lock:
            self.assertEqual(manager.cache["b"]["ids"], ["b"])
            self.assertEqual(manager.cache["b"]["invalid"], [["a", "b"]])


if __name__ == "__main__":
    unittest.main()
