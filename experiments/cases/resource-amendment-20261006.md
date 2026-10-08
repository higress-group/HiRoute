# Measured usage and resource amendment — 2026-10-06

This amendment follows the [parallel amendment](parallel-amendment-20261006.md).
The parent proposed a larger resource allowance so the requested complete
workflows can run, and proceeds with that recommended default unless the user
specifies otherwise. The original 500,000-token allowance was an executor
planning choice, not a budget set by the user. No article quality assessment or
blind-model unmasking preceded this change.

The observed first writing draft consumed 391,429 cumulative input/output
tokens, before three remaining editorial stages. The first audit session
exceeded the old allowance during an unfinished turn. These observations show
that the initial allowance is restrictive for this native-agent workflow. They
do not establish model quality or a monetary price.

Use a common **3,000,000 measured cumulative input plus output tokens per
replicate**, checked at each turn boundary, for both experiments and all arms.
Keep the twelve-minute turn, forty-minute writing, fifty-minute audit and
original 17:53 CST overall deadlines. Do not reset time or omit earlier usage.
Record the allowance applying when a turn starts, and keep earlier stop records
at their original allowance. Never replay completed model turns or add editorial
rounds because of this amendment. A previously budget-stopped replicate may
resume only its never-executed stages in the same conversation and within the
original time limit, with the previous stop record preserved.

An unfinished native turn can lack `turn.completed` while its native rollout
already records cumulative `token_count` usage for finished upstream requests.
Use the latest such counter for the same thread at or before the turn's actual
end as an observed lower bound. Preserve its source and timestamp; do not use a
later stage's counters. Subsequent deltas subtract the previously observed
cumulative value, so the partial work is not counted twice. Unreported usage
from an interrupted last request remains unknown. A decreasing cumulative
counter requires investigation, not a negative charge or silent reset.

The original harness ignored these partial counters and incorrectly started
`F-1` stage 3 after stage 2 crossed the then-active 500,000-token boundary. That
stage was interrupted. Preserve its requests, partial artifacts and all known
usage as a harness protocol violation. Its partial grader results cannot count
as a completed primary cell or evidence that Flash failed the task. Freeze this
replicate's remaining cells without replaying it; report the missing comparison
explicitly alongside the other registered replicate. Do not hide its cost.

The regression replays the real usage records without model calls: stage 2's
observed cumulative count is 1,346,629, while the later stage raises it to
1,432,065. The corrected end-time filter distinguishes them and subsequent
deltas do not recount the earlier work. Raw observations, previous result files
and recovery records remain in the private evidence directory. Neither this
repair nor the parallel controller changes HiRoute product behavior or any
frozen task prompt, source material, model setting or grader assertion.
