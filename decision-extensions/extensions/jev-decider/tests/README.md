# Decision extension behavior tests

From the extension directory, run `python -m unittest discover -s tests -v` with the declared
dependencies installed. Tests use the real HTTP handler and loopback upstream; no real keys or paid calls.

`test_decision.py` owns definition compilation, independent category refinements, request-local question
mapping, strict probability validation, selected-path isolation, raw Score normalization, current-input
preservation, whole-history trimming, single-group branches and reported usage. `decision_cases.py`
loads the model-routing examples from the canonical current v1 cases.

`test_server.py` owns the actual HTTP producer/consumer: exact supplied definitions, one authenticated
provider call including optional assessment, duplicate/unknown fields and output bounds, timeout including
queue wait, inbound authentication and key-file settings. Old mode/threshold/policy-file settings no longer
control decision semantics; prompts and thresholds belong to the published HiRoute plan.

These tests prove the extension contract. Real HiRoute publication, multi-turn routing, cancellation,
actual model execution and session observation are separate gateway/product acceptance gates.
