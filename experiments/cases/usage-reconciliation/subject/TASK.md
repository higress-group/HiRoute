# Model-usage audit utility

Build a small Python 3.11+ standard-library utility for a team's model gateway.
The source is synthetic attempt-level usage events, not prices or a money bill.
Users need known usage to remain distinguishable from missing usage.

Implement usage_audit.py. Requirements arrive in six user requests and accumulate;
preserve earlier behavior. You may add local tests and explanatory documentation.
There is no network, package installation or external service dependency. Use
ordinary files and tools in this directory. Read example.jsonl for the basic data
shape, but do not assume its values cover all valid data.

Do not claim tests you have not run. Finish each request with a concise account
of the changes and your own verification. The assignment is to implement the
current released requirements, not to predict future ones.
